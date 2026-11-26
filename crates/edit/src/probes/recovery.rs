//! The recovery accounting check (REC-01): a save's holdings counted once,
//! without writing or changing it.

use crate::recovery::{Facts, Holding, inspect, seat_holder};
use sim::inventory::{COLOUR_SLOT, Catalog, Item, ItemFacts, PARTY, slots};

/// The accounting of a save that holds the nine holder kinds the accounting
/// tells apart: the party, a character carrying, a character wearing, the
/// area's floor, a container, a store's stock, an area creature, an area
/// object that should not hold anything, and a holder nothing answers for.
/// Every item is checked by id against ground truth after the save has been
/// written and read back.
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
    // The area's objects that hold things: a container, a store, a creature
    // the party has no character for, and a door that should hold nothing.
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
        // movement state and a fight for exactly those.
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
    let door = save.snapshot.take_id().0;
    let mate = save.snapshot.take_id().0;
    save.snapshot.entities.extend([
        made(chest, sim::module::ObjectKind::Placeable, false),
        made(store, sim::module::ObjectKind::Store, false),
        made(creature, sim::module::ObjectKind::Creature, true),
        made(door, sim::module::ObjectKind::Door, false),
        made(mate, sim::module::ObjectKind::Creature, true),
    ]);
    // A companion the party table knows whose sheet is not in the save: it is
    // still a character, and what it wears is still worn by a character.
    save.script.party.members.push(9);
    save.script.party.available.insert(
        9,
        sim::script::party::Npc {
            template: "c3".into(),
            object: Some(mate),
            kept: None,
            template_data: None,
        },
    );
    // A save numbers its items from `FIRST_ITEM`, the first id the sim gives.
    let first = sim::inventory::FIRST_ITEM;
    let (gone, far) = (0x7d00_9999, 0x7d00_8888);
    let area = sim::script::AREA.0;
    for (n, held) in [
        item("medpac", PARTY, None, 2, false, 0, Vec::new()),
        item(
            "saber",
            player,
            Some(slots::RIGHT_WEAPON),
            1,
            false,
            0,
            vec!["crystal".into()],
        ),
        item("carried", player, None, 1, false, 0, Vec::new()),
        item("not_in_catalog", gone, None, 1, false, 0, Vec::new()),
        item("k1", PARTY, None, 1, false, 2, Vec::new()),
        item("loose", area, None, 1, false, 0, Vec::new()),
        item("loose", chest, None, 3, false, 0, Vec::new()),
        item("stock", store, None, 0, true, 0, Vec::new()),
        item(
            "worn",
            creature,
            Some(slots::HANDS),
            1,
            false,
            0,
            Vec::new(),
        ),
        item("ghost", PARTY, None, 1, false, 0, Vec::new()),
        item("loose", player, None, 1, false, 0, Vec::new()),
        item("worn", door, None, 1, false, 0, Vec::new()),
        item("worn", far, Some(slots::HANDS), 1, false, 0, Vec::new()),
        item("loose", store, None, 1, false, 0, Vec::new()),
        item("worn", mate, Some(slots::HANDS), 1, false, 0, Vec::new()),
    ]
    .into_iter()
    .enumerate()
    {
        save.script.items.insert(first + n as u32, held);
    }
    // The item lying on the area's floor, and the one whose template only the
    // save's carried facts know. The third floor listing is one the save kept
    // after the item was picked up: it is still in the party's hands.
    let lying = sim::inventory::Lying {
        at: [1.0, 2.0, 0.0],
        yaw: 0.0,
    };
    save.script
        .floor
        .insert(first + 5, ("lev_m04aa".into(), lying));
    save.script
        .floor
        .insert(first + 10, ("lev_m04aa".into(), lying));
    // And one the save kept listed after the item was sold to a store: the
    // store holds it now, so the store answers for it.
    save.script
        .floor
        .insert(first + 13, ("lev_m04aa".into(), lying));
    save.script
        .known
        .insert("ghost".into(), ItemFacts::plain("ghost"));

    let truth: &[(u32, Holding, Facts)] = &[
        (first, Holding::PartyLoose, Facts::Catalog),
        (first + 1, Holding::Equipped, Facts::Catalog),
        (first + 2, Holding::Carried, Facts::Catalog),
        (first + 3, Holding::Unknown, Facts::Absent),
        (first + 4, Holding::PartyLoose, Facts::Catalog),
        (first + 5, Holding::Floor, Facts::Catalog),
        (first + 6, Holding::Container, Facts::Catalog),
        (first + 7, Holding::Store, Facts::Catalog),
        (first + 8, Holding::Creature, Facts::Catalog),
        (first + 9, Holding::PartyLoose, Facts::Carried),
        (first + 10, Holding::Carried, Facts::Catalog),
        (first + 11, Holding::AreaObject, Facts::Catalog),
        (first + 12, Holding::Unknown, Facts::Catalog),
        (first + 13, Holding::Store, Facts::Catalog),
        (first + 14, Holding::Equipped, Facts::Catalog),
    ];
    // Writing the save and reading it back is the same accounting: the report
    // is taken from the decoded bytes, not from the tree in hand.
    let bytes = save.encode();
    let back = sim::Save::decode(&bytes).map_err(|why| format!("decode: {why}"))?;
    let round_trip = back == save;
    let before = back.clone();
    let back_bytes = back.encode();
    let back_stamp = back.script.items.stamp();
    let report = inspect(&back, Some(&catalog));
    // Reading one item by its number is part of the same read-only promise.
    let read =
        crate::recovery::held_item(&back, first + 1).is_some_and(|held| held.template == "saber");
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
    let read_only =
        back == before && back.encode() == back_bytes && back.script.items.stamp() == back_stamp;
    let named = truth.len() == report.items.len();
    // The door holding an item is the one anomaly the fixture asks for.
    let flags = report.ambiguities
        == vec![format!(
            "item {} is held by object {}, which does not hold items",
            first + 11,
            door
        )];
    let totals = read
        && round_trip
        && read_only
        && named
        && wrong.is_empty()
        && flags
        && report.units == 17
        && report.infinite == 1
        && report.party_loose == 3
        && report.carried == 2
        && report.equipped == 2
        && report.floor == 1
        && report.containers == 1
        && report.store == 2
        && report.creatures == 1
        && report.area_objects == 1
        && report.unknown == 2
        && report.kotor_rows == 1
        && report.tsl_slots == 1
        && report.colours == 1
        && report.facts_carried == 1
        && report.facts_absent == 1
        && report.unknown_holders == vec![far, gone];
    // No catalog means nothing about the facts: they are not claimed absent.
    let blind = inspect(&back, None);
    let blind_facts = blind.facts_absent == 0
        && blind.facts_carried == 0
        && blind.items.iter().all(|row| row.facts == Facts::NotChecked);
    if !totals || !blind_facts || !wrong.is_empty() {
        return Err(format!(
            "read {read}, round trip {round_trip}, read only {named}, wrong {wrong:?}, flags {flags}, items {}, units {}, infinite {}, party {}, carried {}, equipped {}, floor {}, containers {}, store {}, creatures {}, area objects {}, unknown {}, rows {}, slots {}, colours {}, facts carried {}, facts absent {}, holders {:?}, ambiguities {:?}",
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
            report.area_objects,
            report.unknown,
            report.kotor_rows,
            report.tsl_slots,
            report.colours,
            report.facts_carried,
            report.facts_absent,
            report.unknown_holders,
            report.ambiguities,
        ));
    }
    Ok(
        "fifteen items counted once by holder (party, carried, equipped, floor, container, store, creature, area object and two unknown holders) and by where their facts came from, after the save was written and read back; the save unchanged in tree, bytes and item stamp".into(),
    )
}
