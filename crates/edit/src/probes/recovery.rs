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
    // A host that takes kind 1 in slot 1 up to level 1, and a part of that kind
    // at level 2: the part could not have been fitted there. A second host
    // takes crystals only, up to level 1, and a crystal of level 3 fits there,
    // because the level rule exempts crystals.
    let mut high_host = ItemFacts::plain("k1_high");
    high_host.slot_kinds = vec![COLOUR_SLOT, 1];
    high_host.slots = 1 << slots::RIGHT_WEAPON;
    high_host.upgrade_level = 1;
    let mut big_power = ItemFacts::plain("big_power");
    big_power.upgrade = Some((0, 1));
    big_power.upgrade_level = 2;
    let mut low_host = ItemFacts::plain("k1_low");
    low_host.slot_kinds = vec![sim::inventory::CRYSTAL];
    low_host.slots = 1 << slots::RIGHT_WEAPON;
    low_host.upgrade_level = 1;
    let mut hi_crystal = ItemFacts::plain("hi_crystal");
    hi_crystal.upgrade = Some((0, sim::inventory::CRYSTAL));
    hi_crystal.upgrade_level = 3;
    // The level rule's boundary: a host of level 2 that takes kind 2 in slot 1,
    // and a part of that kind at exactly level 2, which is a legal fit. And a
    // host whose own template starts with a part of a level the host does not
    // take, because an item made of that template begins holding it.
    let mut equal_host = ItemFacts::plain("k1_equal");
    equal_host.slot_kinds = vec![COLOUR_SLOT, 2];
    equal_host.slots = 1 << slots::RIGHT_WEAPON;
    equal_host.upgrade_level = 2;
    let mut equal_part = ItemFacts::plain("equal_part");
    equal_part.upgrade = Some((0, 2));
    equal_part.upgrade_level = 2;
    let mut born_host = ItemFacts::plain("k1_born");
    born_host.slot_kinds = vec![COLOUR_SLOT, 1];
    born_host.slots = 1 << slots::RIGHT_WEAPON;
    born_host.upgrade_level = 1;
    born_host.parts = vec![String::new(), "big_power".into()];
    // The same birth shape with one more slot: the template names its part at
    // index 1, so a save that holds that part at index 2 did not get it from the
    // template, and the bench could not have fitted it above the host's level.
    let mut born_two = ItemFacts::plain("k1_born2");
    born_two.slot_kinds = vec![COLOUR_SLOT, 1, 1];
    born_two.slots = 1 << slots::RIGHT_WEAPON;
    born_two.upgrade_level = 1;
    born_two.parts = vec![String::new(), "big_power".into(), String::new()];
    // A host whose template starts it holding something that is no upgrade at
    // all: `make_item` copies the template's parts with no upgrade test, so that
    // part is in the save because the template says it is.
    let mut born_three = ItemFacts::plain("k1_born3");
    born_three.slot_kinds = vec![COLOUR_SLOT, 1];
    born_three.slots = 1 << slots::RIGHT_WEAPON;
    born_three.upgrade_level = 1;
    born_three.parts = vec![String::new(), "medpac".into()];
    let mut listed = ItemFacts::plain("k1_listed");
    listed.upgrades = vec![sim::inventory::properties::Upgrade {
        row: 1,
        kind: 0,
        template: "crystal".into(),
        properties: Vec::new(),
        rows: 0,
    }];
    // A colour crystal the bench can really fit: `fit` takes the colour route
    // only for a crystal that recolours a hilt other than the host, so a crystal
    // that recolours the host itself goes to the slot route, which needs an
    // upgrade of the colour slot's kind.
    let mut crystal = ItemFacts {
        recolours: vec!["saber".into()],
        ..ItemFacts::plain("crystal")
    };
    crystal.upgrade = Some((0, COLOUR_SLOT));
    // The same for the hilt with two colour slots: its own crystal recolours
    // that hilt and nothing else, so fitting it fills a slot rather than
    // changing the hilt.
    let mut crystal_three = ItemFacts {
        recolours: vec!["saber_three".into()],
        ..ItemFacts::plain("crystal_three")
    };
    crystal_three.upgrade = Some((0, COLOUR_SLOT));
    let alt_crystal = ItemFacts {
        recolours: vec!["saber_alt".into()],
        ..ItemFacts::plain("alt_crystal")
    };
    // Two more colour-slot tenants, both of which the colour route could not
    // have installed: one recolours a hilt this module does not catalogue at
    // all, and one recolours a catalogued hilt of another base item. The host of
    // the second is `alien_host`, whose base item is 7 where `other_hilt`'s is
    // 4, because a crystal recolours the hilts of its own base item and no
    // others. A list of recoloured hilts that is merely non-empty would call
    // both of them colour parts.
    let far_crystal = ItemFacts {
        recolours: vec!["nowhere_hilt".into()],
        ..ItemFacts::plain("far_crystal")
    };
    let alien_crystal = ItemFacts {
        recolours: vec!["other_hilt".into()],
        ..ItemFacts::plain("alien_crystal")
    };
    let other_hilt = ItemFacts {
        base_item: 4,
        ..ItemFacts::plain("other_hilt")
    };
    let mut alien_host = ItemFacts {
        base_item: 7,
        ..ItemFacts::plain("k1_alien")
    };
    alien_host.slot_kinds = vec![COLOUR_SLOT];
    alien_host.slots = 1 << slots::RIGHT_WEAPON;
    let catalog = Catalog::new([
        medpac,
        saber,
        saber_alt,
        saber_two,
        saber_three,
        mixed,
        power,
        high_host,
        big_power,
        low_host,
        hi_crystal,
        equal_host,
        equal_part,
        born_host,
        born_two,
        born_three,
        listed,
        crystal,
        crystal_three,
        alt_crystal,
        far_crystal,
        alien_crystal,
        other_hilt,
        alien_host,
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
    // The puppet's body is also an object of the area, and a creature at that:
    // an accounting that asked the area what the id was before asking the party
    // would call its equipment creature gear.
    save.snapshot
        .entities
        .push(made(puppet, ObjectKind::Creature, true));
    // A puppet that has not joined the party is one of the area's creatures: its
    // body answers as a creature and what it wears is a creature's holding, not a
    // character's.
    let stray = save.snapshot.take_id().0;
    save.snapshot
        .entities
        .push(made(stray, ObjectKind::Creature, true));
    save.script.party.puppets.insert(
        1,
        sim::script::party::Puppet {
            template: "pup1".into(),
            object: Some(stray),
            owner: Some(0),
            joined: false,
        },
    );
    let (gone, far) = (0x7d00_9999, 0x7d00_8888);
    let area = sim::script::AREA.0;
    let row1 = sim::inventory::upgrade_bit(1).ok_or("no row 1")?;
    // template, holder, worn slot, stack, never-spent, KOTOR rows, fitted parts
    type Stock = (&'static str, u32, Option<u8>, u16, bool, u32, Vec<String>);
    let goods: [Stock; 46] = [
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
        // filled: the colour sign has to look at both. The crystal is this
        // hilt's own, so the bench fitted it into the slot instead of making a
        // colour swap.
        (
            "saber_three",
            PARTY,
            None,
            1,
            false,
            0,
            vec![String::new(), "crystal_three".into()],
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
        // 32: worn by a puppet who has joined the party, whose body is also an
        // area creature.
        ("worn", puppet, Some(slots::BELT), 1, false, 0, Vec::new()),
        // 33: a part of the kind the slot takes, but of a level the host does
        // not take.
        (
            "k1_high",
            PARTY,
            None,
            1,
            false,
            0,
            vec![String::new(), "big_power".into()],
        ),
        // 34: a colour slot holding something that recolours nothing.
        ("saber", PARTY, None, 1, false, 0, vec!["medpac".into()]),
        // 35: a crystal above the host's own level, which the level rule lets
        // through because it is a crystal.
        (
            "k1_low",
            PARTY,
            None,
            1,
            false,
            0,
            vec!["hi_crystal".into()],
        ),
        // 36: a part at exactly the level its host takes, which is a legal fit.
        (
            "k1_equal",
            PARTY,
            None,
            1,
            false,
            0,
            vec![String::new(), "equal_part".into()],
        ),
        // 37: a part above the host's level that the host's own template starts
        // with, so it came with the item rather than being fitted on the bench.
        (
            "k1_born",
            PARTY,
            None,
            1,
            false,
            0,
            vec![String::new(), "big_power".into()],
        ),
        // 38: worn by a puppet that has not joined the party, which is one of the
        // area's creatures.
        (
            "worn",
            stray,
            Some(slots::LEFT_WEAPON),
            1,
            false,
            0,
            Vec::new(),
        ),
        // 39 and 40: a colour slot holding a crystal whose recolours list is not
        // empty but names no hilt this swap could use: one hilt the module does
        // not catalogue, one hilt of another base item. Neither is an upgrade of
        // the colour slot's kind either, so the part is unexplained and the
        // colour is unsettled.
        (
            "saber",
            PARTY,
            None,
            1,
            false,
            0,
            vec!["far_crystal".into()],
        ),
        (
            "k1_alien",
            PARTY,
            None,
            1,
            false,
            0,
            vec!["alien_crystal".into()],
        ),
        // 41: a part of a level the host does not take, held at an index the
        // host's own template does not name it at: it came neither with the
        // template nor off the bench.
        (
            "k1_born2",
            PARTY,
            None,
            1,
            false,
            0,
            vec![String::new(), String::new(), "big_power".into()],
        ),
        // 42: a colour slot holding a crystal that recolours a sibling hilt of
        // the same base item. The engine's colour route would have spent it on
        // the hilt and never filled the slot, so this save claims a fitted part
        // it cannot have fitted. The colour itself still reads as settled.
        (
            "saber_three",
            PARTY,
            None,
            1,
            false,
            0,
            vec![String::new(), "crystal".into()],
        ),
        // 43: a colour slot holding an upgrade of another kind entirely, which
        // no route puts in a colour slot.
        ("saber", PARTY, None, 1, false, 0, vec!["power".into()]),
        // 44: a part that is no upgrade at all, held at the index the host's own
        // template names it at, so it came with the item.
        (
            "k1_born3",
            PARTY,
            None,
            1,
            false,
            0,
            vec![String::new(), "medpac".into()],
        ),
        // 45: the above-level part of item 41 at the index its template does
        // name it at: that one came with the item.
        (
            "k1_born2",
            PARTY,
            None,
            1,
            false,
            0,
            vec![String::new(), "big_power".into(), String::new()],
        ),
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
            // The colour slot holds something nothing describes, so the colour
            // is not settled in favour of a swap.
            Colour::Unknown,
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
        // 33: a part of the kind the slot takes, above the host's level.
        row(
            first + 33,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            1,
            1,
            0,
        ),
        // 34: a colour slot holding something that recolours nothing.
        row(
            first + 34,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::Unknown,
            1,
            0,
            1,
            1,
            0,
        ),
        // 35: a crystal above the host's level, which the rule exempts.
        row(
            first + 35,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            1,
            0,
            0,
        ),
        // 36: a part at exactly its host's level: the boundary of the level rule,
        // and a legal fit.
        row(
            first + 36,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            1,
            0,
            0,
        ),
        // 37: a part above the host's level that came with the host's template.
        row(
            first + 37,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            1,
            0,
            0,
        ),
        // 39 and 40: a colour slot holding a crystal that recolours a hilt this
        // module cannot use for colour, so the colour is unsettled and the part
        // is one nobody can explain.
        row(
            first + 39,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::Unknown,
            1,
            0,
            1,
            1,
            0,
        ),
        row(
            first + 40,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::Unknown,
            1,
            0,
            1,
            1,
            0,
        ),
        // 41: a part above its host's level at an index its template does not
        // name it at.
        row(
            first + 41,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            1,
            1,
            0,
        ),
        // 42: a colour slot holding a crystal the colour route would have spent
        // on the hilt: the colour reads as settled, the part is unexplained.
        row(
            first + 42,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::Fitted,
            1,
            0,
            1,
            1,
            0,
        ),
        // 43: a colour slot holding an upgrade of the wrong kind.
        row(
            first + 43,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::Unknown,
            1,
            0,
            1,
            1,
            0,
        ),
        // 44 and 45: parts that came with the host's template, at the index the
        // template names them at, so neither is an anomaly.
        row(
            first + 44,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            1,
            0,
            0,
        ),
        row(
            first + 45,
            Holding::PartyLoose,
            Facts::Catalog,
            Colour::None,
            1,
            0,
            1,
            0,
            0,
        ),
        // 38: a puppet that has not joined is one of the area's creatures.
        row(
            first + 38,
            Holding::Creature,
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
            "item {} has the part big_power in slot 1, whose level 2 is above the level 1 its host takes",
            first + 33
        ),
        format!(
            "item {} has the part medpac in slot 0, which is not an upgrade this module or the save describes",
            first + 34
        ),
        format!(
            "item {} has the part far_crystal in slot 0, which is not an upgrade this module or the save describes",
            first + 39
        ),
        format!(
            "item {} has the part alien_crystal in slot 0, which is not an upgrade this module or the save describes",
            first + 40
        ),
        format!(
            "item {} has the part big_power in slot 2, whose level 2 is above the level 1 its host takes",
            first + 41
        ),
        format!(
            "item {} has the part crystal in colour slot 1, which would recolour its host instead of filling the slot",
            first + 42
        ),
        format!(
            "item {} has the part power in slot 0, whose kind 1 is not the kind 255 its host declares",
            first + 43
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
    // The colour tenants are read from the items in the save, not from the
    // fixture table, and each is held to the engine's own two conditions for
    // putting a part in a colour slot. `State::fit` takes the colour route when
    // the part's facts name a hilt of the host's base item other than the host,
    // and that route changes the hilt's template and hands the crystal back; what
    // is left to the slot route has to be an upgrade whose kind is the colour
    // slot's kind. The tenants the report calls unexplained have to fail one of
    // those two conditions, which is what makes them anomalies rather than fitted
    // colour parts.
    let colour_tenants: Vec<(u32, String, String)> = back
        .script
        .items
        .iter()
        .flat_map(|(id, held)| {
            let Some(host) = catalog.get(&held.template) else {
                return Vec::new();
            };
            host.slot_kinds
                .iter()
                .enumerate()
                .filter(|(_, kind)| **kind == COLOUR_SLOT)
                .filter_map(|(at, _)| {
                    held.fitted
                        .get(at)
                        .filter(|part| !part.is_empty())
                        .map(|part| (*id, part.clone(), host.template.clone()))
                })
                .collect()
        })
        .collect();
    let tenant_could_be_fitted = |part: &str, host: &str| {
        catalog.get(part).is_some_and(|part_facts| {
            catalog.get(host).is_some_and(|host_facts| {
                part_facts
                    .upgrade
                    .is_some_and(|(_, kind)| kind == COLOUR_SLOT)
                    && catalog
                        .recolour(part_facts, host_facts.base_item)
                        .is_none_or(|made| made == host_facts.template)
            })
        })
    };
    let fitted_tenants: Vec<u32> = colour_tenants
        .iter()
        .filter(|(_, part, host)| tenant_could_be_fitted(part, host))
        .map(|(id, _, _)| *id)
        .collect();
    let unfit_tenants: Vec<u32> = colour_tenants
        .iter()
        .filter(|(_, part, host)| !tenant_could_be_fitted(part, host))
        .map(|(id, _, _)| *id)
        .collect();
    // Nine of the save's items hold a part in a colour slot. Three of them are
    // parts a fit could have placed there. Six are not: one is described nowhere,
    // one recolours nothing, two recolour hilts this module cannot use for colour,
    // one recolours a sibling hilt instead of its host, and one is an upgrade of
    // another kind entirely.
    let tenants_as_expected = fitted_tenants == vec![first + 1, first + 21, first + 28]
        && unfit_tenants
            == vec![
                first + 22,
                first + 34,
                first + 39,
                first + 40,
                first + 42,
                first + 43,
            ];
    let tenant_count = colour_tenants.len();
    let read_only =
        back == before && back.encode() == back_bytes && back.script.items.stamp() == back_stamp;
    let named = truth.len() == report.items.len();
    // The two puppets round-trip with the flags the accounting depends on.
    let puppets_kept = back.script.party.puppets.get(&0).is_some_and(|p| p.joined)
        && back.script.party.puppets.get(&1).is_some_and(|p| !p.joined);
    // The puppet's body is an object of the area and a creature at that, in the
    // save as read back: that is the classification the accounting has to lose
    // to the party table, and it is here so the collision is proven and not
    // merely asserted.
    let puppet_is_a_creature = back.snapshot.entities.iter().any(|entity| {
        entity.id == sim::SimId(puppet) && entity.kind == sim::Kind::Object(ObjectKind::Creature, 1)
    });
    let flags = report.ambiguities == wanted;
    // The helper that names a seat's body has to agree with the save's own
    // seat table, which is what the ground truth above was read from.
    let seat_ok = seat_holder(&back, sim::Seat(0)) == Some(sim::SimId(player));
    let totals = read
        && round_trip
        && read_only
        && named
        && tenant_count == 9
        && tenants_as_expected
        && wrong.is_empty()
        && flags
        && seat_ok
        && puppet_is_a_creature
        && puppets_kept
        && report.units == 49
        && report.infinite == 1
        && report.party_loose == 24
        && report.carried == 2
        && report.equipped == 6
        && report.floor == 1
        && report.containers == 2
        && report.store == 3
        && report.creatures == 3
        && report.area_objects == 1
        && report.unknown == 4
        && report.kotor_rows == 3
        && report.tsl_slots == 21
        && report.parts_unresolved == 11
        && report.rows_unlisted == 1
        && report.colours == 5
        && report.colour_fitted == 4
        && report.colour_by_template == 1
        && report.colour_unknown == 6
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
    // The sentence this test reports is built from what the report actually
    // says, and the gate below insists that the sentence names the counts the
    // accounting was checked against. A summary that drifts from the accounting
    // fails the test instead of quietly describing something else.
    let summary = format!(
        "{} items counted once by holder (party, carried, equipped, floor, container, store, creature, area object, three unknown holders, and the chests, stores and creatures left in a played module), by where their own template's facts came from, by how the save shows a colour part, by the units in the stack, and by the rows and slots fitted and whether anyone describes them and whether the host declares the slot and takes the part's kind and level; {} named anomalies; after the save was written and read back, the save unchanged in tree, bytes and item stamp",
        report.items.len(),
        report.ambiguities.len()
    );
    let summary_named_by_the_report = report_summary_matches(&summary, 46, 17);
    if !totals || !blind_facts || !summary_named_by_the_report || !wrong.is_empty() {
        return Err(format!(
            "read {read}, round trip {round_trip}, read only {read_only}, all items named {named}, colour tenants as expected {tenants_as_expected}, colour tenants counted {tenant_count}, seat holder {seat_ok}, flags {flags}, blind {blind_facts}, summary {summary_named_by_the_report}, wrong {wrong:?}, items {}, units {}, infinite {}, party {}, carried {}, equipped {}, floor {}, containers {}, store {}, creatures {}, area objects {}, unknown {}, rows {}, slots {}, parts {}, rows unlisted {}, colours {} ({} fitted, {} by template, {} unknown), facts carried {}, facts absent {}, holders {:?}, ambiguities {:?}",
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
    Ok(summary)
}

/// The summary has to say the two counts the gate checked, in its own words.
/// The counts are written out here rather than taken from the report, so a
/// report that counted differently and a summary that agreed with it both fail.
fn report_summary_matches(summary: &str, items: usize, anomalies: usize) -> bool {
    summary.starts_with(&format!("{items} items counted once by holder"))
        && summary.contains(&format!("{anomalies} named anomalies"))
        && summary.contains("the save unchanged in tree, bytes and item stamp")
}
