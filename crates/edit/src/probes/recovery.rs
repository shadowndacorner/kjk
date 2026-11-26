//! The recovery accounting check (REC-01): a save's holdings counted once,
//! without writing or changing it.

use crate::recovery::{Facts, Holding, inspect, seat_holder};
use sim::inventory::{COLOUR_SLOT, Catalog, Item, ItemFacts, PARTY, slots};

/// A save that holds every kind of holder: the party, a character carrying
/// and wearing, the area's floor, a container, a store's stock, a creature
/// and one holder nothing answers for. The accounting is checked item by item
/// after the save is written and read back.
pub fn accounting() -> Result<String, String> {
    let (_, mut save) = sim::fixtures::kitchen_sink();
    let player = seat_holder(&save, sim::Seat(0)).ok_or("no first seat")?.0;
    let mut medpac = ItemFacts::plain("medpac");
    medpac.stacking = 10;
    let mut saber = ItemFacts::plain("saber");
    saber.slot_kinds = vec![COLOUR_SLOT];
    saber.slots = 1 << slots::RIGHT_WEAPON;
    let catalog = Catalog::new([
        medpac,
        saber,
        ItemFacts::plain("k1"),
        ItemFacts::plain("carried"),
        ItemFacts::plain("loose"),
        ItemFacts::plain("stock"),
        ItemFacts::plain("worn"),
    ]);
    let item = |template: &str,
                holder: u32,
                slot: Option<u8>,
                stack: u16,
                infinite: bool,
                upgrades: u32,
                fitted: Vec<String>| {
        Item {
            template: template.into(),
            holder,
            slot,
            stack,
            charges: 0,
            droppable: true,
            plot: false,
            identified: true,
            infinite,
            non_equippable: false,
            upgrades,
            fitted,
        }
    };
    // The area's objects that hold things: a container, a store and a
    // creature the party has no character for.
    let body = save
        .snapshot
        .entities
        .iter()
        .find(|entity| matches!(entity.kind, sim::Kind::Avatar(_)))
        .copied()
        .ok_or("the fixture has no player body")?;
    let made = |id, kind, actor: bool| sim::EntityState {
        id: sim::SimId(id),
        kind: sim::Kind::Object(kind, 1),
        position: body.position,
        velocity: [0.0; 3],
        // Players and creatures are characters, and a save carries a
        // movement state for exactly those.
        player: actor.then_some(body.player).flatten(),
        retarget_ms: 0,
        life_ms: None,
        going: None,
        object: Some(sim::ObjectState::default()),
        fighter: actor.then(|| sim::combat::fighter::Fighter::fresh(10, 0)),
        sheet: None,
        missile: None,
    };
    let chest = save.snapshot.take_id().0;
    let store = save.snapshot.take_id().0;
    let creature = save.snapshot.take_id().0;
    save.snapshot.entities.extend([
        made(chest, sim::module::ObjectKind::Placeable, false),
        made(store, sim::module::ObjectKind::Store, false),
        made(creature, sim::module::ObjectKind::Creature, true),
    ]);
    // A save's entities are written in id order, so the fixture keeps them
    // there.
    save.snapshot.entities.sort_by_key(|entity| entity.id.0);
    let area = 0x7b00_0001;
    for (id, held) in [
        (1, item("medpac", PARTY, None, 2, false, 0, Vec::new())),
        (
            2,
            item(
                "saber",
                player,
                Some(slots::RIGHT_WEAPON),
                1,
                false,
                0,
                vec!["crystal".into()],
            ),
        ),
        (3, item("carried", player, None, 1, false, 0, Vec::new())),
        (
            4,
            item("not_in_catalog", 0x7d00_9999, None, 1, false, 0, Vec::new()),
        ),
        (5, item("k1", PARTY, None, 1, false, 2, Vec::new())),
        (6, item("loose", area, None, 1, false, 0, Vec::new())),
        (7, item("loose", chest, None, 3, false, 0, Vec::new())),
        (8, item("stock", store, None, 0, true, 0, Vec::new())),
        (
            9,
            item(
                "worn",
                creature,
                Some(slots::HANDS),
                1,
                false,
                0,
                Vec::new(),
            ),
        ),
    ] {
        save.script.items.insert(id, held);
    }
    // The item lying on the area's floor, and the one whose template only
    // the save's carried facts know.
    save.script.floor.insert(
        6,
        (
            "loose".into(),
            sim::inventory::Lying {
                at: [1.0, 2.0, 0.0],
                yaw: 0.0,
            },
        ),
    );
    save.script
        .known
        .insert("ghost".into(), ItemFacts::plain("ghost"));
    save.script
        .items
        .insert(10, item("ghost", PARTY, None, 1, false, 0, Vec::new()));

    let truth: &[(u32, Holding, Facts)] = &[
        (1, Holding::PartyLoose, Facts::Catalog),
        (2, Holding::Equipped, Facts::Catalog),
        (3, Holding::Carried, Facts::Catalog),
        (4, Holding::Unknown, Facts::Absent),
        (5, Holding::PartyLoose, Facts::Catalog),
        (6, Holding::Floor, Facts::Catalog),
        (7, Holding::Container, Facts::Catalog),
        (8, Holding::Store, Facts::Catalog),
        (9, Holding::Creature, Facts::Catalog),
        (10, Holding::PartyLoose, Facts::Carried),
    ];
    // Writing the save and reading it back is the same accounting: the
    // report is taken from the decoded bytes, not from the tree in hand.
    let bytes = save.encode();
    let stamp = save.script.items.stamp();
    let back = sim::Save::decode(&bytes).map_err(|why| format!("decode: {why}"))?;
    let before = save.clone();
    let report = inspect(&back, Some(&catalog));
    let read_back = back == save;
    let counted = report.items.len() == truth.len();
    let mut wrong: Vec<String> = Vec::new();
    for (id, holding, facts) in truth {
        let Some(row) = report.items.iter().find(|row| row.id == *id) else {
            wrong.push(format!("item {id} is not in the report"));
            continue;
        };
        if row.holding != *holding {
            wrong.push(format!(
                "item {id} held by {:?}, expected {holding:?}",
                row.holding
            ));
        }
        if row.facts != *facts {
            wrong.push(format!(
                "item {id} facts {:?}, expected {facts:?}",
                row.facts
            ));
        }
    }
    let read_only = save == before && save.encode() == bytes && save.script.items.stamp() == stamp;
    // Reading one item by its number is part of the same read-only promise.
    let read = crate::recovery::held_item(&back, 2).is_some_and(|held| held.template == "saber");
    let totals = read
        && read_back
        && counted
        && read_only
        && wrong.is_empty()
        && report.units == 12
        && report.infinite == 1
        && report.party_loose == 3
        && report.carried == 1
        && report.equipped == 1
        && report.floor == 1
        && report.containers == 1
        && report.store == 1
        && report.creatures == 1
        && report.unknown == 1
        && report.kotor_rows == 1
        && report.tsl_slots == 1
        && report.colours == 1
        && report.facts_carried == 1
        && report.facts_absent == 1
        && report.unknown_holders == vec![0x7d00_9999]
        && report.ambiguities.is_empty();
    // No catalog means nothing about the facts: they are not claimed absent.
    let blind = inspect(&back, None);
    let blind_facts = blind.facts_absent == 0
        && blind.facts_carried == 0
        && blind.items.iter().all(|row| row.facts == Facts::NotChecked);
    if !totals || !blind_facts || !wrong.is_empty() {
        return Err(format!(
            "read {}, read back {}, read only {}, wrong {wrong:?}, items {}, units {}, infinite {}, party {}, carried {}, equipped {}, floor {}, containers {}, store {}, creatures {}, unknown {}, rows {}, slots {}, colours {}, facts carried {}, facts absent {}, holders {:?}, ambiguities {:?}, blind {:?}",
            read,
            read_back,
            read_only,
            report.items.len(),
            report.units,
            report.infinite,
            report.party_loose,
            report.carried,
            report.equipped,
            report.floor,
            report.containers,
            report.store,
            report.creatures,
            report.unknown,
            report.kotor_rows,
            report.tsl_slots,
            report.colours,
            report.facts_carried,
            report.facts_absent,
            report.unknown_holders,
            report.ambiguities,
            blind.items.iter().map(|row| row.facts).collect::<Vec<_>>(),
        ));
    }
    Ok(
        "ten items counted once by holder (party, carried, equipped, floor, container, store, creature and one unknown holder) and by where their facts came from, after the save was written and read back; the save unchanged in tree, bytes and item stamp".into(),
    )
}
