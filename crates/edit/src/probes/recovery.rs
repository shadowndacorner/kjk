//! The recovery accounting check (REC-01): a save's holdings counted once,
//! without writing or changing it.

use crate::recovery::{Colour, Facts, Holding, inspect, seat_holder};
use sim::inventory::{COLOUR_SLOT, Catalog, Item, ItemFacts, PARTY, slots};
use sim::module::ObjectKind;

/// The accounting of a save that holds every holder kind the accounting tells
/// apart: the party carrying loose, a character carrying, a character wearing,
/// the area's floor, a container, a store's stock, an area creature, an area
/// object that should not hold anything, and holders nothing answers for. It
/// also holds objects left in a module the party has already played, a
/// companion kept while away, an item on the area's books that is not lying on
/// the floor, a stack of three identical sabers with a crystal in each, parts
/// and rows nobody describes, a colour swap that changed the hilt's own
/// template, an item listed twice, and a stack of none. It also holds a hilt
/// with two colour slots whose second one is filled, a part in a slot its host
/// declares that is no upgrade at all, a part of a kind its host's slot does
/// not take, a part in a slot the host's facts do not declare, an item held by
/// a wanderer, and a puppet who has joined the party and wears something.
///
/// Every item is checked by id against ground truth after the save has been
/// written and read back, and the anomalies are checked as a list, not as a
/// count.
pub fn accounting() -> Result<String, String> {
    let (_, mut save) = sim::fixtures::kitchen_sink();
    // The first seat's own body, read from the save and not from the helper
    // under test: the accounting is judged against the save, and the helper is
    // checked against it separately.
    let player = save
        .snapshot
        .entities
        .iter()
        .find(|entity| entity.kind == sim::Kind::Avatar(sim::Seat(0)))
        .ok_or("the fixture has no first seat")?
        .id
        .0;
    // The tables: a medpac that stacks to ten, two sabers with a colour slot,
    // a hilt whose colour is carried by its own template, a KOTOR 1 host whose
    // upgrade rows are listed and one whose rows are not, and the parts.
    let mut medpac = ItemFacts::plain("medpac");
    medpac.stacking = 10;
    let mut saber = ItemFacts::plain("saber");
    saber.slot_kinds = vec![COLOUR_SLOT];
    saber.slots = 1 << slots::RIGHT_WEAPON;
    let mut saber_alt = ItemFacts::plain("saber_alt");
    saber_alt.slot_kinds = vec![COLOUR_SLOT];
    saber_alt.slots = 1 << slots::RIGHT_WEAPON;
    let mut saber_two = ItemFacts::plain("saber_two");
    saber_two.slot_kinds = vec![COLOUR_SLOT];
    saber_two.slots = 1 << slots::RIGHT_WEAPON;
    let mut saber_three = ItemFacts::plain("saber_three");
    saber_three.slot_kinds = vec![COLOUR_SLOT, COLOUR_SLOT];
    saber_three.slots = 1 << slots::RIGHT_WEAPON;
    let mut mixed = ItemFacts::plain("k1_slots");
    mixed.slot_kinds = vec![COLOUR_SLOT, 0];
    mixed.slots = 1 << slots::RIGHT_WEAPON;
    let mut power = ItemFacts::plain("power");
    power.upgrade = Some((0, 1));
    let mut listed = ItemFacts::plain("k1_listed");
    listed.upgrades = vec![sim::inventory::properties::Upgrade {
        row: 1,
        kind: 0,
        template: "crystal".into(),
        properties: Vec::new(),
        rows: 0,
    }];
    let crystal = ItemFacts {
        recolours: vec!["saber".into()],
        ..ItemFacts::plain("crystal")
    };
    let alt_crystal = ItemFacts {
        recolours: vec!["saber_alt".into()],
        ..ItemFacts::plain("alt_crystal")
    };
    let catalog = Catalog::new([
        medpac,
        saber,
        saber_alt,
        saber_two,
        saber_three,
        mixed,
        power,
        listed,
        crystal,
        alt_crystal,
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
        made(chest, ObjectKind::Placeable, false),
        made(store, ObjectKind::Store, false),
        made(creature, ObjectKind::Creature, true),
        made(door, ObjectKind::Door, false),
        made(mate, ObjectKind::Creature, true),
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
    // A companion kept while away: the party table knows its body only as the
    // `from` of its kept record.
    let away = save.snapshot.take_id().0;
    let sheet = sim::combat::sheet::Sheet::commoner();
    save.script.party.available.insert(
        7,
        sim::script::party::Npc {
            template: "c7".into(),
            object: None,
            kept: Some(sim::script::party::Kept {
                from: away,
                sheet,
                hp: i32::from(sheet.max_hp) * 100,
                fp: 0,
                worn: Some([0; sim::inventory::slots::COUNT as usize]),
                look: sim::combat::Look::default(),
            }),
            template_data: None,
        },
    );
    // The modules the party has already played. Their chests, stores and
    // creatures are not in this area, but the items they hold still are.
    let (left_chest, left_store, left_creature) = (
        save.snapshot.take_id().0,
        save.snapshot.take_id().0,
        save.snapshot.take_id().0,
    );
    let left = save
        .snapshot
        .place
        .clone()
        .ok_or("the fixture is not in a module")?;
    save.snapshot.stash = sim::Stash::new(vec![sim::Stashed {
        place: sim::Place {
            module: "lev_left".into(),
            ..left
        },
        entities: vec![
            made(left_chest, ObjectKind::Placeable, false),
            made(left_store, ObjectKind::Store, false),
            made(left_creature, ObjectKind::Creature, true),
        ],
        spawned: Vec::new(),
        roster: None,
    }]);
    // A save numbers its items from `FIRST_ITEM`, the first id the sim gives.
    let first = sim::inventory::FIRST_ITEM;
    // A wanderer is an entity the save names, and it is not an object that
    // holds items: an item held by it is unresolved, but the save does answer
    // for the holder's id.
    let wanderer = save.snapshot.take_id().0;
    save.snapshot.entities.push(sim::EntityState {
        id: sim::SimId(wanderer),
        kind: sim::Kind::Wanderer,
        position: body.position,
        velocity: [0.0; 3],
        player: None,
        retarget_ms: 0,
        life_ms: None,
        going: None,
        object: None,
        fighter: None,
        sheet: None,
        missile: None,
    });
    // A TSL puppet who has joined the party wears things under its own object
    // id, which only the party table names.
    let puppet = save.snapshot.take_id().0;
    save.script.party.puppets.insert(
        0,
        sim::script::party::Puppet {
            template: "pup0".into(),
            object: Some(puppet),
            owner: Some(0),
            joined: true,
        },
    );
    let (gone, far) = (0x7d00_9999, 0x7d00_8888);
    let area = sim::script::AREA.0;
    let row1 = sim::inventory::upgrade_bit(1).ok_or("no row 1")?;
    // template, holder, worn slot, stack, never-spent, KOTOR rows, fitted parts
    type Stock = (&'static str, u32, Option<u8>, u16, bool, u32, Vec<String>);
    let goods: [Stock; 33] = [
        // 0: the party carries two medpacs.
        ("medpac", PARTY, None, 2, false, 0, Vec::new()),
        // 1: the player wears a saber with a colour crystal fitted in it.
        (
            "saber",
            player,
            Some(slots::RIGHT_WEAPON),
            1,
            false,
            0,
            vec!["crystal".into()],
        ),
        // 2: the player carries one thing and wears two others (26 and 27).
        ("carried", player, None, 1, false, 0, Vec::new()),
        // 3: a holder nothing answers for, and a template nothing describes.
        ("not_in_catalog", gone, None, 1, false, 0, Vec::new()),
        // 4: a KOTOR 1 host that says a row is fitted, which its facts do not list.
        ("k1", PARTY, None, 1, false, row1, Vec::new()),
        // 5: lying on the area's floor.
        ("loose", area, None, 1, false, 0, Vec::new()),
        // 6: in a container, three of them.
        ("loose", chest, None, 3, false, 0, Vec::new()),
        // 7: a store's stock, which is never spent.
        ("stock", store, None, 0, true, 0, Vec::new()),
        // 8: worn by a creature of the area.
        (
            "worn",
            creature,
            Some(slots::HANDS),
            1,
            false,
            0,
            Vec::new(),
        ),
        // 9: a template only the save's carried facts know.
        ("ghost", PARTY, None, 1, false, 0, Vec::new()),
        // 10: picked up, though the save still lists it on the floor.
        ("loose", player, None, 1, false, 0, Vec::new()),
        // 11: held by a door, which does not hold items.
        ("worn", door, None, 1, false, 0, Vec::new()),
        // 12: worn by a holder nothing answers for.
        ("worn", far, Some(slots::HANDS), 1, false, 0, Vec::new()),
        // 13: sold to a store, though the save still lists it on the floor.
        ("loose", store, None, 1, false, 0, Vec::new()),
        // 14: worn by the unsheeted companion.
        ("worn", mate, Some(slots::HANDS), 1, false, 0, Vec::new()),
        // 15 and 16: in a chest and a store left in a played module.
        ("loose", left_chest, None, 1, false, 0, Vec::new()),
        ("loose", left_store, None, 1, false, 0, Vec::new()),
        // 17: carried by a creature left in a played module.
        ("loose", left_creature, None, 1, false, 0, Vec::new()),
        // 18: worn by the companion kept while away.
        ("worn", away, Some(slots::HANDS), 1, false, 0, Vec::new()),
        // 19: held by the area, but not listed as lying there.
        ("loose", area, None, 1, false, 0, Vec::new()),
        // 20: a hilt whose colour is carried by its own template.
        ("saber_alt", PARTY, None, 1, false, 0, Vec::new()),
        // 21: three identical sabers in one stack, each with a crystal fitted.
        ("saber", PARTY, None, 3, false, 0, vec!["crystal".into()]),
        // 22: a fitted part neither the module nor the save describes.
        (
            "saber_two",
            PARTY,
            None,
            1,
            false,
            0,
            vec!["not_a_part".into()],
        ),
        // 23: a KOTOR 1 row its host's facts do list.
        ("k1_listed", PARTY, None, 1, false, row1, Vec::new()),
        // 24: one item holding both a KOTOR 1 row and a TSL slot.
        (
            "k1_listed",
            PARTY,
            None,
            1,
            false,
            row1,
            vec!["crystal".into()],
        ),
        // 25: a stack of none.
        ("medpac", PARTY, None, 0, false, 0, Vec::new()),
        // 26 and 27: two items in the player's hand slot.
        (
            "carried",
            player,
            Some(slots::HANDS),
            1,
            false,
            0,
            Vec::new(),
        ),
        ("worn", player, Some(slots::HANDS), 1, false, 0, Vec::new()),
        // 28: a hilt with two colour slots, the first empty and the second
        // filled: the colour sign has to look at both.
        (
            "saber_three",
            PARTY,
            None,
            1,
            false,
            0,
            vec![String::new(), "crystal".into()],
        ),
        // 29: a part in a slot the host declares, where the part is described
        // but is no upgrade at all.
        (
            "k1_slots",
            PARTY,
            None,
            1,
            false,
            0,
            vec![String::new(), "medpac".into()],
        ),
        // 30: a part whose kind is not the kind the host's slot takes.
        (
            "k1_slots",
            PARTY,
            None,
            1,
            false,
            0,
            vec![String::new(), "power".into()],
        ),
        // 31: held by a wanderer.
        ("medpac", wanderer, None, 1, false, 0, Vec::new()),
        // 32: worn by a puppet who has joined the party.
        ("worn", puppet, Some(slots::BELT), 1, false, 0, Vec::new()),
    ];
    for (n, (template, holder, slot, stack, infinite, upgrades, fitted)) in
        goods.into_iter().enumerate()
    {
        save.script.items.insert(
            first + n as u32,
            item(template, holder, slot, stack, infinite, upgrades, fitted),
        );
    }
    // The item lying on the area's floor, and the one whose template only the
    // save's carried facts know. The third floor listing is one the save kept
    // after the item was picked up: it is still in the party's hands.
    let lying = sim::inventory::Lying {
        at: [1.0, 2.0, 0.0],
        yaw: 0.0,
    };
    for id in [first + 5, first + 10, first + 13] {
        save.script.floor.insert(id, ("lev_m04aa".into(), lying));
    }
    save.script
        .known
        .insert("ghost".into(), ItemFacts::plain("ghost"));
    // The screen's copy of one item that is already in the authority's table:
    // it is the same item, and it is counted once.
    save.snapshot.goods.items.push((
        first + 2,
        item("carried", player, None, 1, false, 0, Vec::new()),
    ));

    // Ground truth, written from the table above: who holds the item, where
    // its own template's facts came from, how the save shows a colour part,
    // how many units it stands for, how many rows and slots are fitted over
    // those units, and how many of them nobody describes.
    #[allow(dead_code)]
    struct Row {
        id: u32,
        holding: Holding,
        facts: Facts,
        colour: Colour,
        units: usize,
        rows: usize,
        slots: usize,
        parts: usize,
        unlisted: usize,
    }
    let row = |id: u32,
               holding: Holding,
               facts: Facts,
               colour: Colour,
               units: usize,
               rows: usize,
               slots: usize,
               parts: usize,
               unlisted: usize| {
        Row {
            id,
            holding,
            facts,
            colour,
            units,
            rows,
            slots,
            parts,
            unlisted,
        }
    };
    let truth: Vec<Row> = vec![
        row(
            first,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::None,
            2,
            0,
            0,
            0,
            0,
        ),
        row(
            first + 1,
            Holding::Equipped,
            Facts::Catalog,
            Colour::Fitted,
            1,
            0,
            1,
            0,
            0,
        ),
        row(
            first + 2,
            Holding::Carried,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            0,
            0,
            0,
        ),
        row(
            first + 3,
            Holding::Unknown,
            Facts::Absent,
            Colour::Unknown,
            1,
            0,
            0,
            0,
            0,
        ),
        row(
            first + 4,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::None,
            1,
            1,
            0,
            0,
            1,
        ),
        row(
            first + 5,
            Holding::Floor,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            0,
            0,
            0,
        ),
        row(
            first + 6,
            Holding::Container,
            Facts::Catalog,
            Colour::None,
            3,
            0,
            0,
            0,
            0,
        ),
        row(
            first + 7,
            Holding::Store,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            0,
            0,
            0,
        ),
        row(
            first + 8,
            Holding::Creature,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            0,
            0,
            0,
        ),
        row(
            first + 9,
            Holding::PartyLoose,
            Facts::Carried,
            Colour::None,
            1,
            0,
            0,
            0,
            0,
        ),
        row(
            first + 10,
            Holding::Carried,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            0,
            0,
            0,
        ),
        row(
            first + 11,
            Holding::AreaObject,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            0,
            0,
            0,
        ),
        row(
            first + 12,
            Holding::Unknown,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            0,
            0,
            0,
        ),
        row(
            first + 13,
            Holding::Store,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            0,
            0,
            0,
        ),
        row(
            first + 14,
            Holding::Equipped,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            0,
            0,
            0,
        ),
        row(
            first + 15,
            Holding::Container,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            0,
            0,
            0,
        ),
        row(
            first + 16,
            Holding::Store,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            0,
            0,
            0,
        ),
        row(
            first + 17,
            Holding::Creature,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            0,
            0,
            0,
        ),
        row(
            first + 18,
            Holding::Equipped,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            0,
            0,
            0,
        ),
        row(
            first + 19,
            Holding::Unknown,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            0,
            0,
            0,
        ),
        row(
            first + 20,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::Template,
            1,
            0,
            0,
            0,
            0,
        ),
        row(
            first + 21,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::Fitted,
            3,
            0,
            3,
            0,
            0,
        ),
        row(
            first + 22,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::Fitted,
            1,
            0,
            1,
            1,
            0,
        ),
        row(
            first + 23,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::None,
            1,
            1,
            0,
            0,
            0,
        ),
        // A KOTOR 1 host with a TSL part fitted: its facts declare no slot at
        // all, so the part is not accounted for.
        row(
            first + 24,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::None,
            1,
            1,
            1,
            1,
            0,
        ),
        // A stack of none still stands for one row of the save's item table.
        row(
            first + 25,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            0,
            0,
            0,
        ),
        row(
            first + 26,
            Holding::Equipped,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            0,
            0,
            0,
        ),
        row(
            first + 27,
            Holding::Equipped,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            0,
            0,
            0,
        ),
        // 28: the colour part sits in the second of two colour slots.
        row(
            first + 28,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::Fitted,
            1,
            0,
            1,
            0,
            0,
        ),
        // 29 and 30: parts the host's slot cannot take.
        row(
            first + 29,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            1,
            1,
            0,
        ),
        row(
            first + 30,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            1,
            1,
            0,
        ),
        // 31: a holder the save names but which cannot hold items.
        row(
            first + 31,
            Holding::Unknown,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            0,
            0,
            0,
        ),
        // 32: a puppet who has joined the party.
        row(
            first + 32,
            Holding::Equipped,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            0,
            0,
            0,
        ),
    ];
    // Every anomaly, in the order the accounting meets them.
    let wanted: Vec<String> = vec![
        format!(
            "item {} carries KOTOR upgrade row 1, which its host's facts do not list",
            first + 4
        ),
        format!(
            "item {} is held by object {}, which does not hold items",
            first + 11,
            door
        ),
        format!(
            "item {} has the part not_a_part in slot 0, which neither the module nor the save describes",
            first + 22
        ),
        format!(
            "item {} has the part crystal in slot 0, which its host's facts do not declare",
            first + 24
        ),
        format!(
            "item {} holds both KOTOR upgrade rows and TSL fitted slots",
            first + 24
        ),
        format!("item {} has a zero stack", first + 25),
        format!(
            "item {} has the part medpac in slot 1, which is not an upgrade this module or the save describes",
            first + 29
        ),
        format!(
            "item {} has the part power in slot 1, whose kind 1 is not the kind 0 its host declares",
            first + 30
        ),
        format!(
            "item {} is held by object {}, which is not an object that holds items",
            first + 31,
            wanderer
        ),
        format!(
            "holder {player} has 2 items equipped in slot {}",
            slots::HANDS
        ),
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
    for want in &truth {
        let Some(row) = report.items.iter().find(|row| row.id == want.id) else {
            wrong.push(format!("item {} is not in the report", want.id));
            continue;
        };
        if row.holding != want.holding {
            wrong.push(format!(
                "item {} held by {:?}, expected {:?}",
                want.id, row.holding, want.holding
            ));
        }
        if row.facts != want.facts {
            wrong.push(format!(
                "item {} facts {:?}, expected {:?}",
                want.id, row.facts, want.facts
            ));
        }
        if row.colour != want.colour {
            wrong.push(format!(
                "item {} colour {:?}, expected {:?}",
                want.id, row.colour, want.colour
            ));
        }
        if row.units != want.units {
            wrong.push(format!(
                "item {} units {}, expected {}",
                want.id, row.units, want.units
            ));
        }
        if row.kotor_rows != want.rows {
            wrong.push(format!(
                "item {} rows {}, expected {}",
                want.id, row.kotor_rows, want.rows
            ));
        }
        if row.tsl_slots != want.slots {
            wrong.push(format!(
                "item {} slots {}, expected {}",
                want.id, row.tsl_slots, want.slots
            ));
        }
        if row.unresolved_parts != want.parts {
            wrong.push(format!(
                "item {} parts nobody describes {}, expected {}",
                want.id, row.unresolved_parts, want.parts
            ));
        }
        if row.unlisted_rows != want.unlisted {
            wrong.push(format!(
                "item {} rows not on its host {}, expected {}",
                want.id, row.unlisted_rows, want.unlisted
            ));
        }
    }
    let read_only =
        back == before && back.encode() == back_bytes && back.script.items.stamp() == back_stamp;
    let named = truth.len() == report.items.len();
    let flags = report.ambiguities == wanted;
    // The helper that names a seat's body has to agree with the save's own
    // seat table, which is what the ground truth above was read from.
    let seat_ok = seat_holder(&back, sim::Seat(0)) == Some(sim::SimId(player));
    let totals = read
        && round_trip
        && read_only
        && named
        && wrong.is_empty()
        && flags
        && seat_ok
        && report.units == 36
        && report.infinite == 1
        && report.party_loose == 12
        && report.carried == 2
        && report.equipped == 6
        && report.floor == 1
        && report.containers == 2
        && report.store == 3
        && report.creatures == 2
        && report.area_objects == 1
        && report.unknown == 4
        && report.kotor_rows == 3
        && report.tsl_slots == 9
        && report.parts_unresolved == 4
        && report.rows_unlisted == 1
        && report.colours == 5
        && report.colour_fitted == 4
        && report.colour_by_template == 1
        && report.colour_unknown == 1
        && report.facts_carried == 1
        && report.facts_absent == 1
        && report.unknown_holders == vec![far, gone, area];
    // No catalog means nothing about the facts, the colour, or whether a part
    // or a row is described: none of it is claimed.
    let blind = inspect(&back, None);
    let blind_facts = blind.facts_absent == 0
        && blind.facts_carried == 0
        && blind.colours == 0
        && blind.colour_unknown == 0
        && blind.parts_unresolved == 0
        && blind.rows_unlisted == 0
        && blind
            .items
            .iter()
            .all(|row| row.facts == Facts::NotChecked && row.colour == Colour::NotChecked)
        && blind.ambiguities
            == vec![
                format!(
                    "item {} is held by object {}, which does not hold items",
                    first + 11,
                    door
                ),
                format!(
                    "item {} holds both KOTOR upgrade rows and TSL fitted slots",
                    first + 24
                ),
                format!("item {} has a zero stack", first + 25),
                format!(
                    "item {} is held by object {}, which is not an object that holds items",
                    first + 31,
                    wanderer
                ),
                format!(
                    "holder {player} has 2 items equipped in slot {}",
                    slots::HANDS
                ),
            ];
    if !totals || !blind_facts || !wrong.is_empty() {
        return Err(format!(
            "read {read}, round trip {round_trip}, read only {read_only}, all items named {named}, seat holder {seat_ok}, flags {flags}, blind {blind_facts}, wrong {wrong:?}, items {}, units {}, infinite {}, party {}, carried {}, equipped {}, floor {}, containers {}, store {}, creatures {}, area objects {}, unknown {}, rows {}, slots {}, parts {}, rows unlisted {}, colours {} ({} fitted, {} by template, {} unknown), facts carried {}, facts absent {}, holders {:?}, ambiguities {:?}",
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
            report.parts_unresolved,
            report.rows_unlisted,
            report.colours,
            report.colour_fitted,
            report.colour_by_template,
            report.colour_unknown,
            report.facts_carried,
            report.facts_absent,
            report.unknown_holders,
            report.ambiguities,
        ));
    }
    Ok(
        "twenty-eight items counted once by holder (party, carried, equipped, floor, container, store, creature, area object, three unknown holders, and the chests, stores and creatures left in a played module), by where their own template's facts came from, by how the save shows a colour part, by the units in the stack, and by the rows and slots fitted and whether anyone describes them; six named anomalies; after the save was written and read back, the save unchanged in tree, bytes and item stamp".into(),
    )
}
