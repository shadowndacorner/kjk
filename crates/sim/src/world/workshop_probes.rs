//! `kjk sim probes --only workbench`: TSL's workbench, lab station and
//! upgrade bench (`inventory::workshop`, `script::workbench`,
//! `script::items::State::assemble`) on the proving ground, with tables,
//! items and scripts of our own; no install.
//!
//! - The workbench: a blaster worth 400 (a price of 20) is broken down with
//!   Repair 10 (half of it: 10 components), a part worth 200 (10) is made of
//!   them, each with the game's sound, then refused for want of
//!   components; a plot item, one worth nothing and one with upgrades
//!   fitted are not broken down; a recipe above the maker's rank is
//!   refused, one far above it unlisted; gloves of Repair +2 make the rank
//!   reach it.
//! - The lab station: only what its table lists outside its last group is
//!   broken down (not a pistol, not an HK part). Its yield goes by the
//!   maker's Treat Injury total as `GetSkillRank` reads it: 10 ranks and
//!   the +1 of the player's Wisdom 12, so a medkit worth 20 chemicals
//!   yields 11. Too few chemicals is its own line.
//! - Components are a count wherever they come: taken from a container;
//!   made on the player (none made, the game's "Acquired" line); made on a
//!   creature outside the party, and among its starting items, its own count;
//!   added to the party's as it joins, its own kept. The blade it carries
//!   is the party's as it joins; the gloves it wears stay on it.
//! - The upgrade bench's Assemble: an upgrade above the item's level
//!   refuses the whole visit; its changes made together; a crystal fits
//!   whatever its level; a worn item its wearer may no longer wear goes to
//!   the party's pack.

use super::SimWorld;
use crate::action::{Action, Fitting, Workbench};
use crate::combat::Sheet;
use crate::input::{Command, Seat, TickInput};
use crate::inventory::properties::{Limit, Property, Upgrade};
use crate::inventory::workshop::{self, Pool, Recipe, Workshop};
use crate::inventory::{COLOUR_SLOT, Catalog, Item, ItemFacts, PARTY, slots};
use crate::notice::{Notice, Screen};
use crate::probe::{Checks, check};
use crate::script::workbench::{ACQUIRED_COMPONENTS, BROKEN, MADE, sound};
use crate::script::{Out, Pending};
use crate::{StepReason, WorldConfig};
use fmt_odyssey::ncs::{Const, Ncs, Op};
use script_ncs::probes::Asm;
use script_ncs::vm::ObjectId;
use std::sync::Arc;

/// The base items: an ordinary one, and the components'.
const PLAIN: u16 = 1;
const COMPONENTS: u16 = 96;

/// The companion the scripts make, its number and tag.
const NPC: i32 = 3;
const COMPANION: &str = "probe_bench_npc";

const CATALOG: &str = r#"
// 0
object CreateItemOnObject(string sItemTemplate, object oTarget = OBJECT_SELF, int nStackSize = 1);
// 1
object GetFirstPC();
// 2
int GetIsObjectValid(object oObject);
// 3
void SetCustomToken(int nCustomTokenNumber, string sTokenValue);
// 4
string IntToString(int nInteger);
// 5
int AddAvailableNPCByTemplate(int nNPC, string sTemplate);
// 6
object SpawnAvailableNPC(int nNPC, location lPosition);
// 7
location GetLocation(object oObject);
// 8
int AddPartyMember(int nNPC, object oCreature);
// 9
object GetObjectByTag(string sTag, int nNth = 0);
"#;

fn call(routine: u16, args: u8) -> Op {
    Op::Action { routine, args }
}

fn int(value: i32) -> Op {
    Op::Const(Const::Int(value))
}

fn text(value: &str) -> Op {
    Op::Const(Const::String(value.as_bytes().to_vec()))
}

/// The item made on the object on top, taken off it, kept valid or not as
/// custom token `n`.
fn made_on_top(asm: &mut Asm, stack: i32, n: i32) {
    asm.op(int(stack))
        .op(Op::CpTopSp {
            offset: -8,
            size: 4,
        })
        .op(text("compont"))
        .op(call(0, 3))
        .op(call(2, 1))
        .op(call(4, 1))
        .op(int(n))
        .op(call(3, 2));
}

/// `probe_pool`: five components made on the player (token 1 whether an
/// item came of it). `probe_hoard`: the companion made available, made
/// beside the player (its two components among its starting items), three
/// made on it at once (token 2). `probe_join`: it joins the party.
fn scripts() -> [(&'static str, Ncs); 3] {
    let mut pool = Asm::default();
    pool.op(call(1, 0));
    made_on_top(&mut pool, 5, 1);
    pool.op(Op::MovSp(-4)).op(Op::Retn);
    let mut hoard = Asm::default();
    hoard
        .op(text(COMPANION))
        .op(int(NPC))
        .op(call(5, 2))
        .op(Op::MovSp(-4));
    hoard
        .op(call(1, 0))
        .op(call(7, 1))
        .op(int(NPC))
        .op(call(6, 2));
    made_on_top(&mut hoard, 3, 2);
    hoard.op(Op::MovSp(-4)).op(Op::Retn);
    let mut join = Asm::default();
    join.op(int(0))
        .op(text(COMPANION))
        .op(call(9, 2))
        .op(int(NPC))
        .op(call(8, 2))
        .op(Op::MovSp(-4))
        .op(Op::Retn);
    [
        ("probe_pool", pool.build()),
        ("probe_hoard", hoard.build()),
        ("probe_join", join.build()),
    ]
}

fn item(template: &str, cost: u32, base_item: u16, plot: bool) -> ItemFacts {
    ItemFacts {
        cost,
        base_item,
        plot,
        stacking: 100,
        ..super::containers::probes::plain(template)
    }
}

/// An upgrade of kind `kind` (`upgradetypes.2da` row) and level `level`, at
/// `upgrade.2da` row 0.
fn part(template: &str, kind: u8, level: u8, properties: Vec<Property>) -> ItemFacts {
    upgrade_part(template, 0, kind, level, properties)
}

/// An upgrade at `upgrade.2da` row `row`, of kind `kind` and level `level`.
fn upgrade_part(
    template: &str,
    row: u16,
    kind: u8,
    level: u8,
    properties: Vec<Property>,
) -> ItemFacts {
    ItemFacts {
        upgrade: Some((row, kind)),
        upgrade_level: level,
        properties,
        ..item(template, 100, PLAIN, false)
    }
}

/// An item of TSL's slots of `kinds`, upgrade level 1, worn in `slot`.
fn host(template: &str, kinds: Vec<u8>, slot: u8) -> ItemFacts {
    ItemFacts {
        upgrade_level: 1,
        slot_kinds: kinds,
        slots: 1 << slot,
        stacking: 1,
        ..item(template, 1000, PLAIN, false)
    }
}

fn catalog() -> Catalog {
    let mut gloves = item("gloves", 100, PLAIN, false);
    gloves.slots = 1 << slots::HANDS;
    gloves.gear.skills[usize::from(workshop::REPAIR)] = 2;
    Catalog::new([
        item("blaster", 400, PLAIN, false),
        item("pistol", 400, PLAIN, false),
        item("part", 200, PLAIN, false),
        item("rifle", 4000, PLAIN, false),
        item("idol", 400, PLAIN, true),
        item("rag", 0, PLAIN, false),
        item("compont", 0, COMPONENTS, true),
        item("medkit", 400, PLAIN, false),
        item("hkpart", 400, PLAIN, false),
        item("vibroblade", 80, PLAIN, false),
        gloves,
        host("scoped", vec![4, 5, 6], slots::RIGHT_WEAPON),
        ItemFacts {
            stacking: 10,
            ..host("survivor", vec![4, 5], slots::RIGHT_WEAPON)
        },
        host(
            "saber",
            vec![0, COLOUR_SLOT, 0, 1, 2, 3],
            slots::RIGHT_WEAPON,
        ),
        part("scope", 4, 1, Vec::new()),
        part("scope2", 4, 2, Vec::new()),
        part("pack", 5, 1, Vec::new()),
        part("crystal", 0, 3, Vec::new()),
        part(
            "lens",
            1,
            1,
            vec![Property::Limit(Limit::Tag("bastila".into()))],
        ),
    ])
}

/// The tables: the workbench's part (Repair 1), rifle (Repair 12: listed to
/// a rank of 4, not made) and an armour of Repair 30 (unlisted); the lab
/// station's medkit (Treat Injury) and an HK part in its last group.
fn workshop() -> Workshop {
    let recipe = |template: &str, skill, base_skill, group| Recipe {
        template: template.into(),
        skill,
        base_skill,
        group,
        level: 0,
        align: 0,
    };
    let bench = vec![
        recipe("part", 1, workshop::REPAIR, 4),
        recipe("rifle", 12, workshop::REPAIR, 4),
        recipe("blaster", 30, workshop::REPAIR, 4),
    ];
    let lab = vec![
        recipe("medkit", 1, workshop::TREAT_INJURY, 0),
        recipe("hkpart", 50, 1, workshop::UNMADE),
    ];
    Workshop {
        tables: [
            (workshop::BENCH.to_string(), bench),
            (workshop::LAB.to_string(), lab),
        ]
        .into(),
        pools: [(COMPONENTS, Pool::Components)].into(),
    }
}

fn world() -> SimWorld {
    let ground = crate::determinism::proving_ground();
    let mut companion =
        crate::module::ObjectData::creature(COMPANION, COMPANION, [0.0; 3], Sheet::dev_soldier());
    let held = |template: &str, stack, slot| crate::inventory::Held {
        template: template.into(),
        stack,
        slot,
        droppable: true,
        infinite: false,
    };
    companion.inventory = vec![
        held("compont", 2, None),
        held("vibroblade", 1, None),
        held("gloves", 1, Some(slots::HANDS)),
    ];
    let library = crate::probe::library(CATALOG, scripts())
        .with_items(catalog())
        .with_workshop(workshop())
        .with_creatures([(COMPANION.to_string(), Arc::new(companion))].into());
    SimWorld::new(WorldConfig::one_seat(), ground.with_scripts(library))
        .expect("the proving ground makes a world")
}

fn colour_world(catalog: Catalog) -> SimWorld {
    let library = crate::probe::library(CATALOG, scripts()).with_items(catalog);
    SimWorld::new(
        WorldConfig::one_seat(),
        crate::determinism::proving_ground().with_scripts(library),
    )
    .expect("the proving ground makes a world")
}

fn colour_catalog() -> Catalog {
    let mut saber = item("saber", 1000, PLAIN, false);
    saber.upgrades = vec![Upgrade {
        row: 1,
        kind: 0,
        template: "crystal".into(),
        properties: Vec::new(),
        rows: 0,
    }];
    let old = ItemFacts {
        recolours: vec!["saber".into()],
        ..part("old_crystal", 0, 0, Vec::new())
    };
    let new = ItemFacts {
        recolours: vec!["saber_red".into()],
        ..part("new_crystal", 0, 0, Vec::new())
    };
    Catalog::new([
        saber,
        item("saber_red", 1000, PLAIN, false),
        old,
        new,
        upgrade_part("crystal", 1, 0, 3, Vec::new()),
        upgrade_part("bench_crystal", 1, 0, 0, Vec::new()),
    ])
}

fn missing_colour_catalog() -> Catalog {
    let new = ItemFacts {
        recolours: vec!["saber_red".into()],
        ..part("new_crystal", 0, 0, Vec::new())
    };
    Catalog::new([
        item("saber", 1000, PLAIN, false),
        item("saber_red", 1000, PLAIN, false),
        new,
    ])
}

fn missing_upgrade_catalog() -> Catalog {
    let mut saber = item("saber", 1000, PLAIN, false);
    saber.upgrades = vec![Upgrade {
        row: 1,
        kind: 0,
        template: "missing".into(),
        properties: Vec::new(),
        rows: 0,
    }];
    Catalog::new([saber])
}

fn ambiguous_colour_catalog() -> Catalog {
    let old1 = ItemFacts {
        recolours: vec!["saber".into()],
        ..part("old1", 0, 0, Vec::new())
    };
    let old2 = ItemFacts {
        recolours: vec!["saber".into()],
        ..part("old2", 0, 0, Vec::new())
    };
    let new = ItemFacts {
        recolours: vec!["saber_red".into()],
        ..part("new_crystal", 0, 0, Vec::new())
    };
    Catalog::new([
        item("saber", 1000, PLAIN, false),
        item("saber_red", 1000, PLAIN, false),
        old1,
        old2,
        new,
    ])
}

fn incompatible_colour_catalog() -> Catalog {
    let mut old_saber = item("saber", 1000, PLAIN, false);
    old_saber.upgrades = vec![Upgrade {
        row: 1,
        kind: 0,
        template: "crystal".into(),
        properties: Vec::new(),
        rows: 0,
    }];
    let mut new_saber = item("saber_red", 1000, PLAIN, false);
    new_saber.upgrades = vec![Upgrade {
        row: 1,
        kind: 0,
        template: "other".into(),
        properties: Vec::new(),
        rows: 0,
    }];
    let old = ItemFacts {
        recolours: vec!["saber".into()],
        ..part("old_crystal", 0, 0, Vec::new())
    };
    let new = ItemFacts {
        recolours: vec!["saber_red".into()],
        ..part("new_crystal", 0, 0, Vec::new())
    };
    Catalog::new([
        old_saber,
        new_saber,
        old,
        new,
        upgrade_part("crystal", 1, 0, 3, Vec::new()),
        upgrade_part("other", 1, 0, 3, Vec::new()),
    ])
}

/// A hilt with an ordinary crystal slot and an ordinary upgrade slot, whose
/// new colour's hilt lacks that upgrade slot's kind. What sits in the slot is
/// a part the session reader really fits there, not a colour part.
fn incompatible_slot_catalog() -> Catalog {
    let old_saber = host("slot_saber", vec![0, 1], slots::RIGHT_WEAPON);
    let new_saber = host("slot_saber_red", vec![0, 2], slots::RIGHT_WEAPON);
    let old = ItemFacts {
        recolours: vec!["slot_saber".into()],
        ..part("old_crystal", 0, 0, Vec::new())
    };
    let new = ItemFacts {
        recolours: vec!["slot_saber_red".into()],
        ..part("new_crystal", 0, 0, Vec::new())
    };
    Catalog::new([
        old_saber,
        new_saber,
        old,
        new,
        part("slot_upgrade", 1, 0, Vec::new()),
    ])
}

/// A hilt whose crystal slot holds a crystal of a level above the hilt's
/// own: TSL's bench lists a crystal of any level, so a colour swap keeps it.
fn colour_level_catalog() -> Catalog {
    let old_saber = host("slot_saber", vec![0, 1], slots::RIGHT_WEAPON);
    let new_saber = host("slot_saber_red", vec![0, 1], slots::RIGHT_WEAPON);
    let old = ItemFacts {
        recolours: vec!["slot_saber".into()],
        ..part("old_crystal", 0, 0, Vec::new())
    };
    let new = ItemFacts {
        recolours: vec!["slot_saber_red".into()],
        ..part("new_crystal", 0, 0, Vec::new())
    };
    Catalog::new([
        old_saber,
        new_saber,
        old,
        new,
        part("hi_crystal", 0, 3, Vec::new()),
        part("slot_upgrade", 1, 0, Vec::new()),
    ])
}

/// A hilt whose fitted part came from an earlier module: this module's
/// catalog has no template for it, only `State::known` does.
fn colour_known_catalog() -> Catalog {
    let old_saber = host("slot_saber", vec![0, 1], slots::RIGHT_WEAPON);
    let new_saber = host("slot_saber_red", vec![0, 1], slots::RIGHT_WEAPON);
    let old = ItemFacts {
        recolours: vec!["slot_saber".into()],
        ..part("old_crystal", 0, 0, Vec::new())
    };
    let new = ItemFacts {
        recolours: vec!["slot_saber_red".into()],
        ..part("new_crystal", 0, 0, Vec::new())
    };
    Catalog::new([old_saber, new_saber, old, new])
}

/// As above, with two carried templates that both answer for one bare
/// resref and describe parts of different levels. The old hilt takes a
/// part of level 3; its new colour's hilt takes only level 1.
fn ambiguous_known_catalog() -> Catalog {
    let old_saber = ItemFacts {
        upgrade_level: 3,
        ..host("slot_saber", vec![0, 1], slots::RIGHT_WEAPON)
    };
    let new_saber = host("slot_saber_red", vec![0, 1], slots::RIGHT_WEAPON);
    let old = ItemFacts {
        recolours: vec!["slot_saber".into()],
        ..part("old_crystal", 0, 0, Vec::new())
    };
    let new = ItemFacts {
        recolours: vec!["slot_saber_red".into()],
        ..part("new_crystal", 0, 0, Vec::new())
    };
    Catalog::new([old_saber, new_saber, old, new])
}

/// A hilt whose fitted part shares its resref with this module's own
/// template, and whose facts were carried from the module the part was
/// fitted in. The two descriptions agree only when the part really is the
/// same. `old_level` is what the old hilt takes and `own_level` what this
/// module's own template of that resref asks.
fn shadowed_known_catalog(old_level: u8, own_level: u8) -> Catalog {
    let old_saber = ItemFacts {
        upgrade_level: old_level,
        ..host("slot_saber", vec![0, 1], slots::RIGHT_WEAPON)
    };
    let new_saber = host("slot_saber_red", vec![0, 1], slots::RIGHT_WEAPON);
    let old = ItemFacts {
        recolours: vec!["slot_saber".into()],
        ..part("old_crystal", 0, 0, Vec::new())
    };
    let new = ItemFacts {
        recolours: vec!["slot_saber_red".into()],
        ..part("new_crystal", 0, 0, Vec::new())
    };
    let own = crate::inventory::own_template("shared_part", "b");
    Catalog::new([
        old_saber,
        new_saber,
        old,
        new,
        part(&own, 1, own_level, Vec::new()),
    ])
}

/// The hilt, its colour crystals and the module's own template of the
/// fitted part's resref, as a module with a hilt of `host_level` and an own
/// part of `part_level` sees them.
fn shadow_module(which: &str, host_level: u8, part_level: u8) -> Catalog {
    let old_saber = ItemFacts {
        upgrade_level: host_level,
        ..host("slot_saber", vec![0, 1], slots::RIGHT_WEAPON)
    };
    let new_saber = host("slot_saber_red", vec![0, 1], slots::RIGHT_WEAPON);
    let old = ItemFacts {
        recolours: vec!["slot_saber".into()],
        ..part("old_crystal", 0, 0, Vec::new())
    };
    let new = ItemFacts {
        recolours: vec!["slot_saber_red".into()],
        ..part("new_crystal", 0, 0, Vec::new())
    };
    let own = crate::inventory::own_template("shared_part", which);
    Catalog::new([
        old_saber,
        new_saber,
        old,
        new,
        part(&own, 1, part_level, Vec::new()),
    ])
}

/// A third shape of the module entered: its catalog has the shared template
/// of the fitted part's resref, with no module's own marking on it.
fn plain_shared_module(level: u8) -> Catalog {
    let old_saber = host("slot_saber", vec![0, 1], slots::RIGHT_WEAPON);
    let new_saber = host("slot_saber_red", vec![0, 1], slots::RIGHT_WEAPON);
    let old = ItemFacts {
        recolours: vec!["slot_saber".into()],
        ..part("old_crystal", 0, 0, Vec::new())
    };
    let new = ItemFacts {
        recolours: vec!["slot_saber_red".into()],
        ..part("new_crystal", 0, 0, Vec::new())
    };
    Catalog::new([
        old_saber,
        new_saber,
        old,
        new,
        part("shared_part", 1, level, Vec::new()),
    ])
}

/// The part a module's own template of `resref` describes, for a probe to
/// say which reading of a fitted part it is judging.
fn own_part(catalog: &Catalog, resref: &str, module: &str) -> ItemFacts {
    catalog
        .get(&crate::inventory::own_template(resref, module))
        .expect("the module's own part")
        .clone()
}

/// The upgrade level a hilt takes.
fn hilt_level(catalog: &Catalog, hilt: &str) -> u8 {
    catalog.get(hilt).expect("the hilt").upgrade_level
}

/// How many crystals this module's catalog names as the hilt template `hilt`'s
/// colour. `fit_inner` refuses a colour swap unless exactly one does, and it
/// does that before it looks at anything fitted.
fn colour_mappings(catalog: &Catalog, hilt: &str) -> usize {
    catalog
        .iter()
        .filter(|crystal| crystal.recolours.iter().any(|made| made == hilt))
        .count()
}

/// What fitting the crystal item `crystal` into the hilt item `hilt` comes to
/// in this module, exactly as `fit_inner` sees it: the templates those two
/// items really hold, resolved through the catalog and the save's carried
/// facts; the crystal's colour route, with its own hilt filtered out; and the
/// count of crystals naming the hilt they hold as their colour. `None` means
/// the fitting is not a colour swap at all, or is one `fit_inner` would refuse
/// before the route. `Some((new hilt, old mappings))` is the hilt the route
/// aims at and how many old colour mappings it finds, which is what decides
/// whether `State::preserves` is reached.
fn colour_swap(
    catalog: &Catalog,
    before: &crate::script::State,
    hilt: u32,
    crystal: u32,
) -> Option<(String, usize)> {
    let (Some(held_hilt), Some(held_part)) = (before.items.get(&hilt), before.items.get(&crystal))
    else {
        return None;
    };
    if held_part.holder != PARTY || held_part.slot.is_some() || hilt == crystal {
        return None;
    }
    let (Some(host), Some(part)) = (
        before.item_facts(catalog, &held_hilt.template),
        before.item_facts(catalog, &held_part.template),
    ) else {
        return None;
    };
    let colour = catalog
        .recolour(part, host.base_item)
        .filter(|template| *template != host.template)?;
    Some((colour.to_string(), colour_mappings(catalog, &host.template)))
}

/// Whether the swap the probe means is the swap that reaches
/// `State::preserves`: a colour route to `new_hilt`, and exactly one crystal
/// naming the hilt the item holds as its colour. A refusal with this true can
/// only come from what is fitted, not from an earlier check in `fit_inner`.
fn swap_reaches_preserves(
    catalog: &Catalog,
    before: &crate::script::State,
    hilt: u32,
    crystal: u32,
    new_hilt: &str,
) -> bool {
    colour_swap(catalog, before, hilt, crystal) == Some((new_hilt.to_string(), 1))
}

/// The save's slot `slot` of the item under `id` holds exactly `part`. This is
/// the value a part description is looked up by, so a probe about a fitted part
/// has to name it.
fn fitted_names(before: &crate::script::State, id: u32, slot: usize, part: &str) -> bool {
    before
        .items
        .get(&id)
        .is_some_and(|held| held.fitted.get(slot).map(String::as_str) == Some(part))
}

/// That a refused swap left what is in play alone in both directions: the
/// crystal it was given is still in the party's hands, and the crystal it
/// would have put back is nowhere. `Out` carries no item ids, so this is how
/// the probes say no item was consumed and none was made.
fn nothing_swapped(world: &SimWorld, given: &str, returned: &str) -> bool {
    count(world, given) == 1 && count(world, returned) == 0
}

/// The KOTOR 1 row `row` of `hilt` names a part this module really has, at
/// that row and kind: the row is not a name floating free of the catalog.
fn row_names_a_part(catalog: &Catalog, hilt: &str, row: u8) -> bool {
    catalog
        .get(hilt)
        .and_then(|facts| facts.takes(row))
        .is_some_and(|upgrade| {
            catalog
                .get(&upgrade.template)
                .is_some_and(|part| part.upgrade == Some((u16::from(row), upgrade.kind)))
        })
}

/// The two descriptions the probe is judging are descriptions of the resref
/// the save slot names. `same` says whether the probe means them to be the
/// same template: two modules' own templates of one resref are different
/// templates, and that is what makes the probe's disagreement real.
fn descriptions_of(a: &ItemFacts, b: &ItemFacts, resref: &str, same: bool) -> bool {
    crate::inventory::resref_of(&a.template) == resref
        && crate::inventory::resref_of(&b.template) == resref
        && (a.template == b.template) == same
}

/// The part could have been fitted in this hilt: the hilt has a slot of the
/// part's kind, and the hilt takes the part's level (a crystal of any level
/// fits, as `fit_inner` rules). A probe that writes a fitted part by hand has
/// to say the part could have got there.
fn hilt_could_take(catalog: &Catalog, hilt: &str, slot: usize, part: &ItemFacts) -> bool {
    let Some(host) = catalog.get(hilt) else {
        return false;
    };
    let Some((_, kind)) = part.upgrade else {
        return false;
    };
    // `fit_inner` tries the colour route before it looks at any slot: a part
    // that recolours another hilt of the same base item is never fitted into a
    // slot at all, so a probe that hands such a part over in `fitted` is
    // describing a state the bench could not have made.
    if catalog
        .recolour(part, host.base_item)
        .is_some_and(|made| made != host.template)
    {
        return false;
    }
    // The slot named has to take the part's kind, and the part's level has to
    // be within what the hilt takes, except for a crystal, which any hilt takes.
    host.slot_kinds.get(slot) == Some(&kind)
        && (kind == crate::inventory::CRYSTAL || part.upgrade_level <= host.upgrade_level)
}

/// Both hilts take `kind` in slot `slot`, so a refusal about what is fitted
/// there cannot be a slot-kind disagreement.
fn both_slots_take(
    catalog: &Catalog,
    old_hilt: &str,
    new_hilt: &str,
    slot: usize,
    kind: u8,
) -> bool {
    [old_hilt, new_hilt].iter().all(|hilt| {
        catalog
            .get(hilt)
            .is_some_and(|facts| facts.slot_kinds.get(slot) == Some(&kind))
    })
}

fn slot_catalog() -> Catalog {
    Catalog::new([
        host("slot_saber", vec![COLOUR_SLOT, 0], slots::RIGHT_WEAPON),
        part("slot_crystal", COLOUR_SLOT, 0, Vec::new()),
        part("slot_crystal2", COLOUR_SLOT, 0, Vec::new()),
    ])
}

fn give_with(
    world: &mut SimWorld,
    catalog: &Catalog,
    template: &str,
    holder: u32,
    slot: Option<u8>,
) -> u32 {
    let mut state = world.script_state().clone();
    let made = state
        .make_item(catalog, template, holder, 1, slot)
        .map_or(0, |id| id.0);
    world.set_script_state(state);
    made
}

fn template_of(world: &SimWorld, id: u32) -> Option<String> {
    world
        .script_state()
        .items
        .get(&id)
        .map(|held| held.template.clone())
}

fn stack_of(world: &SimWorld, template: &str) -> u16 {
    world
        .script_state()
        .items
        .values()
        .filter(|held| held.template == template)
        .map(|held| held.stack)
        .sum()
}

/// The party's whole pack: every item it carries, with its number, in id
/// order. What a workbench transaction leaves, stated exactly.
fn party_goods(world: &SimWorld) -> Vec<(u32, Item)> {
    world
        .script_state()
        .items
        .held_by(PARTY)
        .map(|(id, held)| (id, held.clone()))
        .collect()
}

/// An expected carried item: `id`, of `template`, carried loose, `stack`
/// units, K1 upgrade bits `upgrades` and TSL `fitted` parts, with the
/// charges and flags a fresh item of the module's catalog gets.
fn carried(id: u32, template: &str, stack: u16, upgrades: u32, fitted: Vec<String>) -> (u32, Item) {
    (
        id,
        Item {
            template: template.to_string(),
            holder: PARTY,
            slot: None,
            stack,
            charges: 0,
            droppable: true,
            plot: false,
            identified: true,
            infinite: false,
            non_equippable: false,
            upgrades,
            fitted,
        },
    )
}

/// Whether everything in the script state other than the items, the item
/// numbers and the carried facts is as `before` left it.
/// The parts of the outcome a successful swap leaves behind, named one by one
/// so a failure says which one went wrong.
fn swap_outcome(
    world: &SimWorld,
    before: &crate::script::State,
    change: (&Out, &Out),
    expected: &Vec<(u32, Item)>,
    next: u32,
    given: &str,
    in_play: usize,
) -> Vec<(&'static str, bool)> {
    let (out, before_out) = change;
    vec![
        ("goods as expected", party_goods(world) == *expected),
        (
            "one item id taken",
            world.script_state().next_item == next + 1,
        ),
        ("the earlier output kept", out_same(out, before_out)),
        ("the given crystal consumed", count(world, given) == 0),
        (
            "the carried facts kept",
            world.script_state().known == before.known,
        ),
        ("items in play as expected", items_in_play(world) == in_play),
        (
            "every other field kept",
            the_rest_unchanged(before, world.script_state()),
        ),
    ]
}

fn the_rest_unchanged(before: &crate::script::State, after: &crate::script::State) -> bool {
    let mut same = after.clone();
    same.items = before.items.clone();
    same.next_item = before.next_item;
    same == *before
}

/// The number of items in play anywhere, held by anyone.
fn items_in_play(world: &SimWorld) -> usize {
    world.script_state().items.values().count()
}

/// Whether two step outputs are the same. Each is destructured field by
/// field, so a field added to `Out` stops compiling here until this
/// function compares it: the pattern has to name it, and the tuples just
/// below are where it gets compared.
fn out_same(a: &Out, b: &Out) -> bool {
    let Out {
        changes,
        notices,
        goals,
        intents,
        area_sound,
        fade,
        world_fade_in,
        video_effect,
        new_module,
        worn_before,
        made,
        deleted,
        destroyed,
        minigame,
        lost,
        statuses,
        feedback,
        assigned,
    } = a;
    (
        changes,
        notices,
        goals,
        intents,
        area_sound,
        fade,
        world_fade_in,
        video_effect,
        new_module,
        worn_before,
        made,
    ) == (
        &b.changes,
        &b.notices,
        &b.goals,
        &b.intents,
        &b.area_sound,
        &b.fade,
        &b.world_fade_in,
        &b.video_effect,
        &b.new_module,
        &b.worn_before,
        &b.made,
    ) && (
        deleted, destroyed, minigame, lost, statuses, feedback, assigned,
    ) == (
        &b.deleted,
        &b.destroyed,
        &b.minigame,
        &b.lost,
        &b.statuses,
        &b.feedback,
        &b.assigned,
    )
}

/// An output with a value in every field a workbench step can leave alone
/// except the minigame course, which no workbench path touches. A refusal
/// has to leave all of it as it was, so the probes compare against this
/// rather than an empty output.
fn seeded_out() -> Out {
    let mut out = Out::default();
    out.intents.insert(
        1,
        crate::script::actions::Intent {
            target: ObjectId(2),
            passive: false,
            talent: None,
        },
    );
    out.area_sound = Some(crate::snapshot::AreaSound {
        music: crate::snapshot::Track {
            state: 1,
            day: Some(3),
            night: None,
            volume: 40,
        },
        battle: crate::snapshot::Track::default(),
        ambient: crate::snapshot::Track {
            state: 2,
            day: None,
            night: Some(5),
            volume: 60,
        },
        rooms: Vec::new(),
    });
    out.fade = Some(crate::snapshot::Fade {
        start_ms: 1_000,
        wait_ms: 100,
        length_ms: 500,
        colour: [0.0, 0.0, 0.0],
        out: true,
        from: 0.25,
        locked: true,
        conversation: false,
    });
    out.world_fade_in = Some(crate::snapshot::WorldFadeIn::AtDialog);
    out.made.push(crate::script::Made {
        id: ObjectId(7),
        facts: Arc::new(crate::module::ObjectData::creature(
            "probe_made",
            "probe_made",
            [0.0; 3],
            Sheet::dev_soldier(),
        )),
        at: [1.0, 2.0, 3.0],
        yaw: 90.0,
        kept: None,
        reach: crate::install::JUMP,
    });
    out.statuses.push(crate::snapshot::StatusKind::Xp(10));
    out.changes.push(crate::script::Change::UseItem {
        who: ObjectId(0),
        row: 0,
    });
    out.notices.push((None, Notice::CannotUpgrade));
    out.goals.insert(
        1,
        crate::script::actions::Goal {
            point: [0.0, 0.0, 0.0],
            range: 1.0,
            run: false,
        },
    );
    out.worn_before
        .insert(1, vec![(0, "saber".into(), 0, Vec::new())]);
    out.video_effect = Some(Some(3));
    out.new_module = Some(crate::script::NewModule {
        by: ObjectId(0),
        module: "x".into(),
        tag: "y".into(),
    });
    out.deleted.push(ObjectId(0));
    out.destroyed.push(ObjectId(0));
    out.lost = true;
    out.feedback.push((ObjectId(0), 0));
    out.assigned.push(1);
    out
}

fn return_atomic_checks() -> Checks {
    let mut checks: Checks = Vec::new();

    let catalog = colour_catalog();
    let mut world = colour_world(catalog.clone());
    let saber = give_with(&mut world, &catalog, "saber", PARTY, None);
    let old = give_with(&mut world, &catalog, "old_crystal", PARTY, None);
    let new = give_with(&mut world, &catalog, "new_crystal", PARTY, None);
    let before = stack_of(&world, "old_crystal");
    let mut state = world.script_state().clone();
    let mut out = Out::default();
    let result = state.fit(&catalog, &mut out, saber, new, None);
    world.set_script_state(state);
    let after = stack_of(&world, "old_crystal");
    let made = after == before + 1
        && template_of(&world, saber).as_deref() == Some("saber_red")
        && stack_of(&world, "new_crystal") == 0
        && world.script_state().items.contains_key(&old);
    checks.push((
        "a colour crystal returns exactly one old crystal and keeps the saber",
        check(
            result.is_ok() && made,
            format!("result {result:?}, old {before}->{after}"),
        ),
    ));

    let mut world = colour_world(catalog.clone());
    let saber = give_with(&mut world, &catalog, "saber", PARTY, None);
    let mut state = world.script_state().clone();
    let mut held = state.items.get_mut(&saber).expect("saber");
    held.upgrades = crate::inventory::upgrade_bit(1).expect("row 1");
    drop(held);
    world.set_script_state(state.clone());
    let mut out = Out::default();
    let result = state.unfit(&catalog, &mut out, saber, 1);
    world.set_script_state(state);
    let item = world.script_state().items.get(&saber);
    let returned = stack_of(&world, "crystal") == 1;
    checks.push((
        "a K1 row removal returns one crystal and keeps the saber",
        check(
            result.is_ok() && item.is_some_and(|held| held.upgrades == 0) && returned,
            format!("result {result:?}, returned {returned}"),
        ),
    ));

    let catalog = slot_catalog();
    let mut world = colour_world(catalog.clone());
    let saber = give_with(&mut world, &catalog, "slot_saber", PARTY, None);
    let mut state = world.script_state().clone();
    let mut held = state.items.get_mut(&saber).expect("slot_saber");
    held.fitted = vec!["slot_crystal".into(), String::new()];
    drop(held);
    world.set_script_state(state.clone());
    let mut out = Out::default();
    let result = state.unfit(&catalog, &mut out, saber, 0);
    world.set_script_state(state);
    let item = world.script_state().items.get(&saber);
    let returned = stack_of(&world, "slot_crystal") == 1;
    checks.push((
        "a TSL slot removal returns one item and keeps the host",
        check(
            result.is_ok()
                && item.is_some_and(|held| held.fitted.iter().all(String::is_empty))
                && returned,
            format!("result {result:?}, returned {returned}"),
        ),
    ));

    let catalog = slot_catalog();
    let mut world = colour_world(catalog.clone());
    let saber = give_with(&mut world, &catalog, "slot_saber", PARTY, None);
    let new = give_with(&mut world, &catalog, "slot_crystal2", PARTY, None);
    let mut state = world.script_state().clone();
    let mut held = state.items.get_mut(&saber).expect("slot_saber");
    held.fitted = vec!["slot_crystal".into(), String::new()];
    drop(held);
    world.set_script_state(state.clone());
    let mut out = Out::default();
    let result = state.fit(&catalog, &mut out, saber, new, Some(0));
    world.set_script_state(state);
    let item = world.script_state().items.get(&saber);
    let returned = stack_of(&world, "slot_crystal") == 1;
    let consumed = stack_of(&world, "slot_crystal2") == 0;
    let fitted = item.is_some_and(|held| held.fitted[0] == "slot_crystal2");
    checks.push((
        "a TSL slot replacement returns the old part and consumes the new one",
        check(
            result.is_ok() && returned && consumed && fitted,
            format!("result {result:?}, returned {returned}, consumed {consumed}, fitted {fitted}"),
        ),
    ));

    let catalog = colour_catalog();
    let mut world = colour_world(catalog.clone());
    let saber = give_with(&mut world, &catalog, "saber", PARTY, None);
    let mut state = world.script_state().clone();
    let mut held = state.items.get_mut(&saber).expect("saber");
    held.stack = 2;
    held.upgrades = crate::inventory::upgrade_bit(1).expect("row 1");
    drop(held);
    world.set_script_state(state.clone());
    let next = world.script_state().next_item;
    let before = world.script_state().clone();
    let row1 = crate::inventory::upgrade_bit(1).expect("row 1");
    let mut out = seeded_out();
    let before_out = out.clone();
    let mut state = world.script_state().clone();
    let result = state.unfit(&catalog, &mut out, saber, 1);
    world.set_script_state(state);
    // The unit that stays wears its crystal; the one split off loses it, and
    // the crystal comes back to the party as an item of its own. Two new
    // items and no output change.
    let expected = vec![
        carried(saber, "saber", 1, row1, Vec::new()),
        carried(
            crate::inventory::FIRST_ITEM + next,
            "saber",
            1,
            0,
            Vec::new(),
        ),
        carried(
            crate::inventory::FIRST_ITEM + next + 1,
            "crystal",
            1,
            0,
            Vec::new(),
        ),
    ];
    let goods = party_goods(&world);
    let goods_match = goods == expected;
    let ids_taken = world.script_state().next_item == next + 2;
    let output_kept = out_same(&out, &before_out);
    let facts_kept = world.script_state().known == before.known;
    let three_in_play = items_in_play(&world) == 3;
    let fields_kept = the_rest_unchanged(&before, world.script_state());
    let exact =
        goods_match && ids_taken && output_kept && facts_kept && three_in_play && fields_kept;
    checks.push((
        "a stacked K1 host is split before a direct removal, leaving an exact bench",
        check(
            result.is_ok() && exact,
            format!(
                "goods as expected {goods_match}, two item ids taken {ids_taken}, earlier output kept {output_kept}, carried facts kept {facts_kept}, three items in play {three_in_play}, every other field kept {fields_kept}, result {result:?}, goods {goods:?}"
            ),
        ),
    ));

    let catalog = colour_catalog();
    let mut world = colour_world(catalog.clone());
    let saber = give_with(&mut world, &catalog, "saber", PARTY, None);
    let crystal = give_with(&mut world, &catalog, "bench_crystal", PARTY, None);
    let mut state = world.script_state().clone();
    let mut held = state.items.get_mut(&saber).expect("saber");
    held.stack = 2;
    drop(held);
    world.set_script_state(state.clone());
    let next = world.script_state().next_item;
    let before = world.script_state().clone();
    let mut out = seeded_out();
    let before_out = out.clone();
    let mut state = world.script_state().clone();
    let result = state.fit(&catalog, &mut out, saber, crystal, None);
    world.set_script_state(state);
    // One unit of the stack gains the crystal and is added back beside the
    // plain one; the crystal the party carried is gone. One new item.
    let expected = vec![
        carried(saber, "saber", 1, 0, Vec::new()),
        carried(
            crate::inventory::FIRST_ITEM + next,
            "saber",
            1,
            row1,
            Vec::new(),
        ),
    ];
    let goods = party_goods(&world);
    let goods_match = goods == expected;
    let id_taken = world.script_state().next_item == next + 1;
    let output_kept = out_same(&out, &before_out);
    let crystal_consumed = count(&world, "bench_crystal") == 0;
    let facts_kept = world.script_state().known == before.known;
    let two_in_play = items_in_play(&world) == 2;
    let fields_kept = the_rest_unchanged(&before, world.script_state());
    let exact = goods_match
        && id_taken
        && output_kept
        && crystal_consumed
        && facts_kept
        && two_in_play
        && fields_kept;
    checks.push((
        "a stacked K1 host is split before a direct fitting, leaving an exact bench",
        check(
            result.is_ok() && exact,
            format!(
                "goods as expected {goods_match}, one item id taken {id_taken}, earlier output kept {output_kept}, bench crystal consumed {crystal_consumed}, carried facts kept {facts_kept}, two items in play {two_in_play}, every other field kept {fields_kept}, result {result:?}, goods {goods:?}"
            ),
        ),
    ));
    checks
}

fn colour_refusal_checks() -> Checks {
    let mut checks: Checks = Vec::new();

    let catalog = missing_colour_catalog();
    let mut world = colour_world(catalog.clone());
    let saber = give_with(&mut world, &catalog, "saber", PARTY, None);
    let new = give_with(&mut world, &catalog, "new_crystal", PARTY, None);
    let before = world.script_state().clone();
    let mut state = world.script_state().clone();
    let mut out = seeded_out();
    let before_out = out.clone();
    let result = state.fit(&catalog, &mut out, saber, new, None);
    let kept = state == before && out_same(&out, &before_out);
    world.set_script_state(state);
    let swapped = nothing_swapped(&world, "new_crystal", "old_crystal");
    // The named condition, read off the items really held: the swap is a
    // colour swap of the hilt's own template `saber` into `saber_red`, and
    // this module's catalog names no crystal as `saber`'s colour.
    let swap = colour_swap(&catalog, &before, saber, new);
    checks.push((
        "a missing old colour mapping refuses without changing the save state or output",
        check(
            swap == Some(("saber_red".to_string(), 0))
                && result == Err(Notice::CannotUpgrade)
                && kept
                && swapped,
            format!("swap {swap:?}, result {result:?}, kept {kept}, nothing swapped {swapped}"),
        ),
    ));

    let catalog = ambiguous_colour_catalog();
    let mut world = colour_world(catalog.clone());
    let saber = give_with(&mut world, &catalog, "saber", PARTY, None);
    let new = give_with(&mut world, &catalog, "new_crystal", PARTY, None);
    let before = world.script_state().clone();
    let mut state = world.script_state().clone();
    let mut out = seeded_out();
    let before_out = out.clone();
    let result = state.fit(&catalog, &mut out, saber, new, None);
    let kept = state == before && out_same(&out, &before_out);
    world.set_script_state(state);
    let swapped = nothing_swapped(&world, "new_crystal", "old_crystal");
    // The named condition, read off the items really held: the swap is a
    // colour swap of `saber` into `saber_red`, and two crystals name the hilt
    // the item holds as the hilt they recolour.
    let swap = colour_swap(&catalog, &before, saber, new);
    checks.push((
        "two old colour mappings for the same saber refuse instead of choosing one",
        check(
            swap == Some(("saber_red".to_string(), 2))
                && result == Err(Notice::CannotUpgrade)
                && kept
                && swapped,
            format!("swap {swap:?}, result {result:?}, kept {kept}, nothing swapped {swapped}"),
        ),
    ));

    let catalog = colour_catalog();
    let mut world = colour_world(catalog.clone());
    let saber = give_with(&mut world, &catalog, "saber", PARTY, None);
    let new = give_with(&mut world, &catalog, "new_crystal", PARTY, None);
    let mut state = world.script_state().clone();
    let mut held = state.items.get_mut(&saber).expect("saber");
    held.upgrades = crate::inventory::upgrade_bit(1).expect("row 1");
    drop(held);
    world.set_script_state(state.clone());
    let before = state.clone();
    let mut out = seeded_out();
    let before_out = out.clone();
    let result = state.fit(&catalog, &mut out, saber, new, None);
    let kept = state == before && out_same(&out, &before_out);
    world.set_script_state(state);
    let swapped = nothing_swapped(&world, "new_crystal", "old_crystal");
    // The named condition: the hilt really carries the K1 power-crystal row,
    // the old hilt's table takes that row with the power crystal, the new
    // colour's hilt takes no such row, and the swap is a colour swap with one
    // old colour mapping, so `preserves` is what refuses.
    let carries_row = before
        .items
        .get(&saber)
        .is_some_and(|held| held.upgrades == crate::inventory::upgrade_bit(1).expect("row 1"));
    let old_takes_row = catalog
        .get("saber")
        .and_then(|facts| facts.takes(1))
        .is_some_and(|upgrade| upgrade.template == "crystal" && upgrade.kind == 0);
    let new_lacks_row = catalog
        .get("saber_red")
        .is_some_and(|facts| facts.takes(1).is_none());
    // And the row's template is a part this module really has at that row.
    let row_names = row_names_a_part(&catalog, "saber", 1);
    let reaches = swap_reaches_preserves(&catalog, &before, saber, new, "saber_red");
    checks.push((
        "a colour swap that cannot keep an installed power crystal refuses",
        check(
            carries_row
                && old_takes_row
                && new_lacks_row
                && row_names
                && reaches
                && result == Err(Notice::CannotUpgrade)
                && kept
                && swapped,
            format!(
                "carries row 1 {carries_row}, old hilt takes it with `crystal` {old_takes_row}, its row names a real part {row_names}, new hilt takes no row 1 {new_lacks_row}, swap reaches `preserves` {reaches}, result {result:?}, kept {kept}, nothing swapped {swapped}"
            ),
        ),
    ));

    let catalog = incompatible_colour_catalog();
    let mut world = colour_world(catalog.clone());
    let saber = give_with(&mut world, &catalog, "saber", PARTY, None);
    let new = give_with(&mut world, &catalog, "new_crystal", PARTY, None);
    let mut state = world.script_state().clone();
    let mut held = state.items.get_mut(&saber).expect("saber");
    held.upgrades = crate::inventory::upgrade_bit(1).expect("row 1");
    drop(held);
    world.set_script_state(state.clone());
    let before = state.clone();
    let mut out = seeded_out();
    let before_out = out.clone();
    let result = state.fit(&catalog, &mut out, saber, new, None);
    let kept = state == before && out_same(&out, &before_out);
    world.set_script_state(state);
    let swapped = nothing_swapped(&world, "new_crystal", "old_crystal");
    // The named condition: both hilts take the same row at the same kind, but
    // the old one asks `crystal` and the new colour's hilt asks `other`, so it
    // is the row's template that disagrees.
    let upgrade_row = |hilt: &str| {
        catalog
            .get(hilt)
            .and_then(|facts| facts.takes(1))
            .map(|upgrade| (upgrade.template.clone(), upgrade.kind))
    };
    let (old_row, new_row) = (upgrade_row("saber"), upgrade_row("saber_red"));
    let other_template = old_row.as_ref().map(|(template, _)| template.as_str()) == Some("crystal")
        && new_row.as_ref().map(|(template, _)| template.as_str()) == Some("other")
        && old_row.as_ref().map(|(_, kind)| *kind) == new_row.as_ref().map(|(_, kind)| *kind);
    let carries_row = before
        .items
        .get(&saber)
        .is_some_and(|held| held.upgrades == crate::inventory::upgrade_bit(1).expect("row 1"));
    // Both rows name parts this module really has, so the difference is in the
    // template the row asks for and not in a name neither catalog knows.
    let rows_name =
        row_names_a_part(&catalog, "saber", 1) && row_names_a_part(&catalog, "saber_red", 1);
    let reaches = swap_reaches_preserves(&catalog, &before, saber, new, "saber_red");
    checks.push((
        "a colour swap with a different K1 row template refuses",
        check(
            other_template
                && carries_row
                && rows_name
                && reaches
                && result == Err(Notice::CannotUpgrade)
                && kept
                && swapped,
            format!(
                "old row {old_row:?}, new row {new_row:?}, both rows name real parts {rows_name}, carries row 1 {carries_row}, swap reaches `preserves` {reaches}, result {result:?}, kept {kept}, nothing swapped {swapped}"
            ),
        ),
    ));

    let catalog = incompatible_slot_catalog();
    let mut world = colour_world(catalog.clone());
    let saber = give_with(&mut world, &catalog, "slot_saber", PARTY, None);
    let new = give_with(&mut world, &catalog, "new_crystal", PARTY, None);
    let mut state = world.script_state().clone();
    let mut held = state.items.get_mut(&saber).expect("slot_saber");
    held.fitted = vec![String::new(), "slot_upgrade".into()];
    drop(held);
    world.set_script_state(state.clone());
    let before = state.clone();
    let mut out = seeded_out();
    let before_out = out.clone();
    let result = state.fit(&catalog, &mut out, saber, new, None);
    let kept = state == before && out_same(&out, &before_out);
    world.set_script_state(state);
    let swapped = nothing_swapped(&world, "new_crystal", "old_crystal");
    // The named condition: the part in slot 1 is really of the kind the old
    // hilt's slot 1 takes, and the new colour's hilt gives slot 1 a different
    // kind, so it is the slot kind that disagrees.
    let fitted_part_kind = catalog
        .get("slot_upgrade")
        .and_then(|facts| facts.upgrade)
        .map(|(_, kind)| kind);
    let old_slot_kind = catalog
        .get("slot_saber")
        .and_then(|facts| facts.slot_kinds.get(1))
        .copied();
    let new_slot_kind = catalog
        .get("slot_saber_red")
        .and_then(|facts| facts.slot_kinds.get(1))
        .copied();
    let slot_kind_differs = fitted_part_kind.is_some()
        && fitted_part_kind == old_slot_kind
        && new_slot_kind.is_some()
        && fitted_part_kind != new_slot_kind;
    let fitted_in_slot = before
        .items
        .get(&saber)
        .is_some_and(|held| held.fitted.get(1).map(String::as_str) == Some("slot_upgrade"));
    let reaches = swap_reaches_preserves(&catalog, &before, saber, new, "slot_saber_red");
    // The part could also have been fitted in the hilt it sits in, and it is
    // within the level the new hilt takes: otherwise a level rule refuses the
    // swap for a reason this probe does not name, and the slot-kind rule could
    // be missing from the code without anyone noticing.
    let origin = catalog.get("slot_upgrade").is_some_and(|facts| {
        hilt_could_take(&catalog, "slot_saber", 1, facts)
            && facts.upgrade_level <= hilt_level(&catalog, "slot_saber_red")
    });
    checks.push((
        "a colour swap with a different TSL slot kind refuses",
        check(
            slot_kind_differs
                && fitted_in_slot
                && origin
                && reaches
                && result == Err(Notice::CannotUpgrade)
                && kept
                && swapped,
            format!(
                "part kind {fitted_part_kind:?}, old slot 1 kind {old_slot_kind:?}, new slot 1 kind {new_slot_kind:?}, fitted there {fitted_in_slot}, part could be fitted there and is within the new hilt's level {origin}, swap reaches `preserves` {reaches}, result {result:?}, kept {kept}, nothing swapped {swapped}"
            ),
        ),
    ));

    let catalog = colour_level_catalog();
    let mut world = colour_world(catalog.clone());
    let saber = give_with(&mut world, &catalog, "slot_saber", PARTY, None);
    let new = give_with(&mut world, &catalog, "new_crystal", PARTY, None);
    let mut state = world.script_state().clone();
    let mut held = state.items.get_mut(&saber).expect("slot_saber");
    held.fitted = vec!["hi_crystal".into(), String::new()];
    drop(held);
    world.set_script_state(state);
    let next = world.script_state().next_item;
    let before = world.script_state().clone();
    let mut out = seeded_out();
    let before_out = out.clone();
    let mut state = world.script_state().clone();
    let result = state.fit(&catalog, &mut out, saber, new, None);
    world.set_script_state(state);
    let expected = vec![
        carried(
            saber,
            "slot_saber_red",
            1,
            0,
            vec!["hi_crystal".to_string(), String::new()],
        ),
        carried(
            crate::inventory::FIRST_ITEM + next,
            "old_crystal",
            1,
            0,
            Vec::new(),
        ),
    ];
    let goods = party_goods(&world);
    // The named condition: the crystal in the hilt's crystal slot is a crystal
    // (`upgradetypes.2da` kind `CRYSTAL`) of a level above what either hilt
    // takes, so only the bench's rule that a crystal of any level fits lets the
    // swap through.
    let crystal_above_both_hilts = catalog.get("hi_crystal").is_some_and(|facts| {
        facts.upgrade == Some((0, crate::inventory::CRYSTAL))
            && facts.upgrade_level > hilt_level(&catalog, "slot_saber")
            && facts.upgrade_level > hilt_level(&catalog, "slot_saber_red")
    }) && before
        .items
        .get(&saber)
        .is_some_and(|held| held.fitted.first().map(String::as_str) == Some("hi_crystal"));
    let reaches = swap_reaches_preserves(&catalog, &before, saber, new, "slot_saber_red")
        && both_slots_take(&catalog, "slot_saber", "slot_saber_red", 0, 0);
    let parts = swap_outcome(
        &world,
        &before,
        (&out, &before_out),
        &expected,
        next,
        "new_crystal",
        2,
    );
    let exact = crystal_above_both_hilts && reaches && parts.iter().all(|(_, ok)| *ok);
    let crystal_level = catalog
        .get("hi_crystal")
        .map(|facts| facts.upgrade_level)
        .unwrap_or(0);
    let old_hilt_level = hilt_level(&catalog, "slot_saber");
    let new_hilt_level = hilt_level(&catalog, "slot_saber_red");
    checks.push((
        "a colour swap keeps a crystal of a level above the hilt's own",
        check(
            result.is_ok() && exact,
            format!(
                "crystal level {crystal_level} above the old hilt's {old_hilt_level} and the new hilt's {new_hilt_level}: {crystal_above_both_hilts}; swap reaches `preserves` {reaches}; outcome {parts:?}; result {result:?}, goods {goods:?}"
            ),
        ),
    ));

    let catalog = colour_known_catalog();
    let mut world = colour_world(catalog.clone());
    let saber = give_with(&mut world, &catalog, "slot_saber", PARTY, None);
    let new = give_with(&mut world, &catalog, "new_crystal", PARTY, None);
    let mut state = world.script_state().clone();
    // TSL's save slots name a fitted part by its bare resref; the carried
    // facts are keyed by its full template.
    let facts = crate::inventory::own_template("off_catalog", "m");
    let carried_part = part(&facts, 1, 0, Vec::new());
    state.known.insert(facts.clone(), carried_part.clone());
    let mut held = state.items.get_mut(&saber).expect("slot_saber");
    held.fitted = vec![String::new(), "off_catalog".into()];
    drop(held);
    world.set_script_state(state);
    let next = world.script_state().next_item;
    let before = world.script_state().clone();
    let mut out = seeded_out();
    let before_out = out.clone();
    let mut state = world.script_state().clone();
    let result = state.fit(&catalog, &mut out, saber, new, None);
    world.set_script_state(state);
    let expected = vec![
        carried(
            saber,
            "slot_saber_red",
            1,
            0,
            vec![String::new(), "off_catalog".to_string()],
        ),
        carried(
            crate::inventory::FIRST_ITEM + next,
            "old_crystal",
            1,
            0,
            Vec::new(),
        ),
    ];
    let goods = party_goods(&world);
    // The named condition: this module's catalog says nothing about the fitted
    // part in either spelling, and the save's own carried facts are the only
    // description of it: a part of the kind both hilts' slot 1 takes.
    let only_the_save_describes_it = catalog.get("off_catalog").is_none()
        && catalog.get(&facts).is_none()
        && before.known.get(&facts) == Some(&carried_part)
        && carried_part.upgrade == Some((0, 1))
        && catalog
            .get("slot_saber")
            .and_then(|facts| facts.slot_kinds.get(1))
            .copied()
            == Some(1)
        && catalog
            .get("slot_saber_red")
            .and_then(|facts| facts.slot_kinds.get(1))
            .copied()
            == Some(1)
        && before
            .items
            .get(&saber)
            .is_some_and(|held| held.fitted.get(1).map(String::as_str) == Some("off_catalog"));
    let reaches = swap_reaches_preserves(&catalog, &before, saber, new, "slot_saber_red");
    let parts = swap_outcome(
        &world,
        &before,
        (&out, &before_out),
        &expected,
        next,
        "new_crystal",
        2,
    );
    let exact = only_the_save_describes_it && reaches && parts.iter().all(|(_, ok)| *ok);
    checks.push((
        "a colour swap keeps a part only State::known describes",
        check(
            result.is_ok() && exact,
            format!(
                "catalog silent about the fitted part {only_the_save_describes_it}, swap reaches `preserves` {reaches}, outcome {parts:?}, result {result:?}, goods {goods:?}"
            ),
        ),
    ));

    let part_a = part(
        &crate::inventory::own_template("one_resref", "a"),
        1,
        0,
        Vec::new(),
    );
    let part_b = part(
        &crate::inventory::own_template("one_resref", "b"),
        1,
        3,
        Vec::new(),
    );
    let catalog = ambiguous_known_catalog();
    let mut world = colour_world(catalog.clone());
    let saber = give_with(&mut world, &catalog, "slot_saber", PARTY, None);
    let new = give_with(&mut world, &catalog, "new_crystal", PARTY, None);
    let mut state = world.script_state().clone();
    for facts in [&part_a, &part_b] {
        state.known.insert(facts.template.clone(), facts.clone());
    }
    let mut held = state.items.get_mut(&saber).expect("slot_saber");
    held.fitted = vec![String::new(), "one_resref".into()];
    drop(held);
    let holds_both = state.known.get(&part_a.template) == Some(&part_a)
        && state.known.get(&part_b.template) == Some(&part_b);
    // Both descriptions come from the save: this module's catalog has no
    // template of the fitted resref at all. They ask the same kind at two
    // levels, and the new hilt takes level 1.
    let catalog_silent = catalog.get("one_resref").is_none();
    let shape = part_a.upgrade == Some((0, 1))
        && part_a.upgrade_level == 0
        && part_b.upgrade == Some((0, 1))
        && part_b.upgrade_level == 3
        && descriptions_of(&part_a, &part_b, "one_resref", false)
        && hilt_could_take(&catalog, "slot_saber", 1, &part_b)
        && hilt_level(&catalog, "slot_saber_red") == 1;
    world.set_script_state(state.clone());
    let before = state.clone();
    let mut out = seeded_out();
    let before_out = out.clone();
    let result = state.fit(&catalog, &mut out, saber, new, None);
    let kept = state == before && out_same(&out, &before_out);
    world.set_script_state(state);
    let swapped = nothing_swapped(&world, "new_crystal", "old_crystal");
    let swap = swap_reaches_preserves(&catalog, &before, saber, new, "slot_saber_red");
    let named = fitted_names(&before, saber, 1, "one_resref");
    let slots = both_slots_take(&catalog, "slot_saber", "slot_saber_red", 1, 1);
    let reaches = swap && named && slots;
    checks.push((
        "two carried templates asking different levels for one fitted resref refuse the colour swap",
        check(
            holds_both
                && catalog_silent
                && shape
                && reaches
                && result == Err(Notice::CannotUpgrade)
                && kept
                && swapped,
            format!(
                "holds both {holds_both}, catalog says nothing {catalog_silent}, a asks {}, b asks {}, hilt 1, swap {swap}, the slot names the part {named}, both hilts take the kind there {slots}, result {result:?}, kept {kept}, nothing swapped {swapped}",
                part_a.upgrade_level, part_b.upgrade_level
            ),
        ),
    ));

    // The old hilt takes a part of level 3 and the carried facts say the
    // fitted part is one; this module's own template of the same resref asks
    // only level 0, and it is the one the catalog answers with.
    let catalog = shadowed_known_catalog(3, 0);
    let part_a = part(
        &crate::inventory::own_template("shared_part", "a"),
        1,
        3,
        Vec::new(),
    );
    let part_b = own_part(&catalog, "shared_part", "b");
    let mut world = colour_world(catalog.clone());
    let saber = give_with(&mut world, &catalog, "slot_saber", PARTY, None);
    let new = give_with(&mut world, &catalog, "new_crystal", PARTY, None);
    let mut state = world.script_state().clone();
    state.known.insert(part_a.template.clone(), part_a.clone());
    let mut held = state.items.get_mut(&saber).expect("slot_saber");
    held.fitted = vec![String::new(), "shared_part".into()];
    drop(held);
    let holds_a = state.known.get(&part_a.template) == Some(&part_a);
    // Only the carried description puts the part above what the new hilt
    // takes: the module's own template of the same resref asks level 0, and
    // it is the one the catalog answers the bare resref with.
    let shape = part_a.upgrade == Some((0, 1))
        && part_a.upgrade_level == 3
        && part_b.upgrade == Some((0, 1))
        && part_b.upgrade_level == 0
        && catalog.get("shared_part") == Some(&part_b)
        && descriptions_of(&part_a, &part_b, "shared_part", false)
        && hilt_could_take(&catalog, "slot_saber", 1, &part_a)
        && hilt_level(&catalog, "slot_saber_red") == 1;
    world.set_script_state(state.clone());
    let before = state.clone();
    let mut out = seeded_out();
    let before_out = out.clone();
    let result = state.fit(&catalog, &mut out, saber, new, None);
    let kept = state == before && out_same(&out, &before_out);
    world.set_script_state(state);
    let swap = swap_reaches_preserves(&catalog, &before, saber, new, "slot_saber_red");
    let named = fitted_names(&before, saber, 1, "shared_part");
    let slots = both_slots_take(&catalog, "slot_saber", "slot_saber_red", 1, 1);
    let reaches = swap && named && slots;
    checks.push((
        "a module's own template of a fitted resref does not stand in for the carried part",
        check(
            holds_a && shape && reaches && result == Err(Notice::CannotUpgrade) && kept,
            format!(
                "holds a {holds_a}, a asks {}, b asks {}, hilt 1, swap {swap}, the slot names the part {named}, both hilts take the kind there {slots}, result {result:?}, kept {kept}",
                part_a.upgrade_level, part_b.upgrade_level
            ),
        ),
    ));

    // Same shape, and the two descriptions ask the same kind and level, so
    // the swap goes ahead.
    let catalog = shadowed_known_catalog(1, 1);
    let part_a = part(
        &crate::inventory::own_template("shared_part", "a"),
        1,
        1,
        Vec::new(),
    );
    let part_b = own_part(&catalog, "shared_part", "b");
    let mut world = colour_world(catalog.clone());
    let saber = give_with(&mut world, &catalog, "slot_saber", PARTY, None);
    let new = give_with(&mut world, &catalog, "new_crystal", PARTY, None);
    let mut state = world.script_state().clone();
    state.known.insert(part_a.template.clone(), part_a.clone());
    let mut held = state.items.get_mut(&saber).expect("slot_saber");
    held.fitted = vec![String::new(), "shared_part".into()];
    drop(held);
    let holds_a = state.known.get(&part_a.template) == Some(&part_a);
    // Both descriptions are really there and really ask the same kind at the
    // same level: the carried one from module a, and the module's own part b
    // for the bare resref the save slot holds.
    let shape = part_a.upgrade == Some((0, 1))
        && part_a.upgrade_level == 1
        && part_b.upgrade == Some((0, 1))
        && part_b.upgrade_level == 1
        && catalog.get("shared_part") == Some(&part_b)
        && descriptions_of(&part_a, &part_b, "shared_part", false)
        && hilt_could_take(&catalog, "slot_saber", 1, &part_a)
        && hilt_level(&catalog, "slot_saber_red") == 1;
    world.set_script_state(state);
    let next = world.script_state().next_item;
    let before = world.script_state().clone();
    let mut out = seeded_out();
    let before_out = out.clone();
    let mut state = world.script_state().clone();
    let result = state.fit(&catalog, &mut out, saber, new, None);
    world.set_script_state(state);
    let expected = vec![
        carried(
            saber,
            "slot_saber_red",
            1,
            0,
            vec![String::new(), "shared_part".to_string()],
        ),
        carried(
            crate::inventory::FIRST_ITEM + next,
            "old_crystal",
            1,
            0,
            Vec::new(),
        ),
    ];
    let goods = party_goods(&world);
    let parts = swap_outcome(
        &world,
        &before,
        (&out, &before_out),
        &expected,
        next,
        "new_crystal",
        2,
    );
    let exact = holds_a && shape && parts.iter().all(|(_, ok)| *ok);
    checks.push((
        "a colour swap goes ahead when the catalog and the carried facts describe the same part",
        check(
            result.is_ok() && exact,
            format!(
                "holds a {holds_a}, descriptions agree {shape}, outcome {parts:?}, result {result:?}, goods {goods:?}",
            ),
        ),
    ));

    // The same shape as a real module change. The part was fitted in the
    // module left, whose own template of its resref asks level 3; the module
    // entered has its own template of that resref asking level 0, so its
    // catalog answers for the bare slot value. The facts of the part fitted
    // in the first module travel with the item, and the swap refuses.
    let left = shadow_module("a", 3, 3);
    let entered = shadow_module("b", 1, 0);
    let mut world = colour_world(left.clone());
    let saber = give_with(&mut world, &left, "slot_saber", PARTY, None);
    let mut state = world.script_state().clone();
    let mut held = state.items.get_mut(&saber).expect("slot_saber");
    held.fitted = vec![String::new(), "shared_part".into()];
    drop(held);
    state.carry_known(&left, &entered);
    let part_a = own_part(&left, "shared_part", "a");
    let part_b = own_part(&entered, "shared_part", "b");
    let holds_a = state.known.get(&part_a.template) == Some(&part_a);
    let hilt = hilt_level(&entered, "slot_saber_red");
    // What each module asks of the fitted resref, and what the module entered
    // answers for the bare resref the save slot holds. Only module a reads the
    // part above what the new hilt takes.
    let shape = part_a.upgrade == Some((0, 1))
        && part_a.upgrade_level == 3
        && part_b.upgrade == Some((0, 1))
        && part_b.upgrade_level == 0
        && entered.get("shared_part") == Some(&part_b)
        && descriptions_of(&part_a, &part_b, "shared_part", false)
        && hilt_could_take(&left, "slot_saber", 1, &part_a)
        && hilt == 1;
    let new = state
        .make_item(&entered, "new_crystal", PARTY, 1, None)
        .expect("the crystal for the swap")
        .0;
    world.set_script_state(state.clone());
    let before = state.clone();
    let mut out = seeded_out();
    let before_out = out.clone();
    let result = state.fit(&entered, &mut out, saber, new, None);
    let kept = state == before && out_same(&out, &before_out);
    world.set_script_state(state);
    let swap = swap_reaches_preserves(&entered, &before, saber, new, "slot_saber_red");
    let named = fitted_names(&before, saber, 1, "shared_part");
    let slots = both_slots_take(&entered, "slot_saber", "slot_saber_red", 1, 1);
    let reaches = swap && named && slots;
    checks.push((
        "a part fitted in the module left keeps its facts across the change, and the swap refuses",
        check(
            holds_a && shape && reaches && result == Err(Notice::CannotUpgrade) && kept,
            format!(
                "holds a {holds_a}, a asks {:?} at {}, b asks {:?} at {}, hilt {hilt}, swap {swap}, the slot names the part {named}, both hilts take the kind there {slots}, result {result:?}, kept {kept}",
                part_a.upgrade,
                part_a.upgrade_level,
                part_b.upgrade,
                part_b.upgrade_level
            ),
        ),
    ));

    // And again into a third module, whose own template of the resref asks
    // level 1, which the new hilt takes: only the facts carried from module a,
    // which ask level 3, can be what refuses the swap here. The state starts
    // from before the first swap, so the crystal is still there and nothing
    // the first swap did can be blamed.
    let third = shadow_module("c", 1, 1);
    let mut state = before.clone();
    state.carry_known(&entered, &third);
    let kept_facts = state.known.get(&part_a.template) == Some(&part_a);
    let holds_b = state.known.get(&part_b.template) == Some(&part_b);
    let has_crystal = state.items.get(&new).is_some();
    let part_c = own_part(&third, "shared_part", "c");
    let shape_c = part_c.upgrade == Some((0, 1))
        && part_c.upgrade_level == 1
        && third.get("shared_part") == Some(&part_c)
        && hilt_level(&third, "slot_saber_red") == 1;
    let before_second = state.clone();
    let mut out = seeded_out();
    let before_out = out.clone();
    let result = state.fit(&third, &mut out, saber, new, None);
    let kept = state == before_second && out_same(&out, &before_out);
    // This hop's own state goes back to a world, so the count of items in
    // play is this probe's and not an earlier probe's.
    world.set_script_state(state.clone());
    let swapped = nothing_swapped(&world, "new_crystal", "old_crystal");
    let swap = swap_reaches_preserves(&third, &before_second, saber, new, "slot_saber_red");
    let named = fitted_names(&before_second, saber, 1, "shared_part");
    let slots = both_slots_take(&third, "slot_saber", "slot_saber_red", 1, 1);
    let reaches = swap && named && slots;
    // Stated here rather than borrowed from the first hop's probe: the part
    // came from module a's hilt, which could take a part of its kind and
    // level, and the two descriptions are of the resref the slot names.
    let origin = hilt_could_take(&left, "slot_saber", 1, &part_a)
        && descriptions_of(&part_a, &part_b, "shared_part", false);
    // And this hop's own premises, not the first hop's: module a asks level 3
    // for the part and module b asks level 0, module b's own template of the
    // resref is the one the module entered answers with, and the hilt entered
    // takes level 1. Without them a competing refusal in module b could stand
    // in for the carried facts here.
    let premises = part_a.upgrade == Some((0, 1))
        && part_a.upgrade_level == 3
        && part_b.upgrade == Some((0, 1))
        && part_b.upgrade_level == 0
        && entered.get("shared_part") == Some(&part_b)
        && hilt_level(&third, "slot_saber_red") == 1;
    checks.push((
        "the facts of the module a part was fitted in survive a second module change",
        check(
            kept_facts
                && holds_b
                && has_crystal
                && origin
                && premises
                && shape_c
                && reaches
                && result == Err(Notice::CannotUpgrade)
                && kept
                && swapped,
            format!(
                "kept_facts {kept_facts}, holds b {holds_b}, crystal {has_crystal}, a's hilt could take the part and both descriptions are of the fitted resref {origin}, a asks 3 and b asks 0 and b answers the resref and the entered hilt takes 1 {premises}, c asks {:?} at level {} {shape_c}, swap {swap}, the slot names the part {named}, both hilts take the kind there {slots}, nothing swapped {swapped}, result {result:?}, kept {kept}",
                part_c.upgrade,
                part_c.upgrade_level
            ),
        ),
    ));

    // The module entered by the second change holds the shared template of
    // the fitted part's resref, with no module's own marking. The facts of
    // the module the part was fitted in still answer, and the swap still
    // refuses.
    let first = shadow_module("a", 3, 3);
    let middle = shadow_module("b", 1, 0);
    let third = plain_shared_module(0);
    let mut world = colour_world(first.clone());
    let saber = give_with(&mut world, &first, "slot_saber", PARTY, None);
    let mut state = world.script_state().clone();
    let mut held = state.items.get_mut(&saber).expect("slot_saber");
    held.fitted = vec![String::new(), "shared_part".into()];
    drop(held);
    state.carry_known(&first, &middle);
    state.carry_known(&middle, &third);
    let wanted_a = own_part(&first, "shared_part", "a");
    let wanted_b = own_part(&middle, "shared_part", "b");
    let holds_a = state.known.get(&wanted_a.template) == Some(&wanted_a);
    let holds_b = state.known.get(&wanted_b.template) == Some(&wanted_b);
    // What each description asks, so that the refusal can only come from the
    // facts carried from module a: module b's own template and the shared
    // template module c reads both ask level 0, and the new hilt takes 1.
    let shared = third
        .get("shared_part")
        .expect("module c's shared template of the part")
        .clone();
    let asks = wanted_a.upgrade == Some((0, 1))
        && wanted_a.upgrade_level == 3
        && wanted_b.upgrade == Some((0, 1))
        && wanted_b.upgrade_level == 0
        && descriptions_of(&wanted_a, &wanted_b, "shared_part", false)
        && hilt_could_take(&first, "slot_saber", 1, &wanted_a)
        && shared.template == "shared_part"
        && shared.upgrade == Some((0, 1))
        && shared.upgrade_level == 0
        && hilt_level(&third, "slot_saber_red") == 1;
    let new = state
        .make_item(&third, "new_crystal", PARTY, 1, None)
        .expect("the crystal for the swap")
        .0;
    world.set_script_state(state.clone());
    let before = state.clone();
    let mut out = seeded_out();
    let before_out = out.clone();
    let result = state.fit(&third, &mut out, saber, new, None);
    let kept = state == before && out_same(&out, &before_out);
    world.set_script_state(state);
    let swapped = nothing_swapped(&world, "new_crystal", "old_crystal");
    let swap = swap_reaches_preserves(&third, &before, saber, new, "slot_saber_red");
    let named = fitted_names(&before, saber, 1, "shared_part");
    let slots = both_slots_take(&third, "slot_saber", "slot_saber_red", 1, 1);
    let reaches = swap && named && slots;
    checks.push((
        "the facts of the module a part was fitted in answer against a later shared template",
        check(
            holds_a
                && holds_b
                && asks
                && reaches
                && result == Err(Notice::CannotUpgrade)
                && kept
                && swapped,
            format!(
                "holds a {holds_a}, holds b {holds_b}, a asks {}, c asks {}, swap {swap}, the slot names the part {named}, both hilts take the kind there {slots}, result {result:?}, kept {kept}, nothing swapped {swapped}",
                wanted_a.upgrade_level, shared.upgrade_level
            ),
        ),
    ));

    // A route through two module changes where every description of the
    // fitted part allows the swap: it was fitted in a module whose own
    // template of its resref asked level 1, both modules entered read that
    // resref at level 0, and the new hilt takes level 1. The three
    // descriptions still differ, so the part's origin is not recoverable,
    // but each of them licenses the swap, so the loadout the player can see
    // is the one they get.
    let left = shadow_module("a", 1, 1);
    let middle = shadow_module("b", 1, 0);
    let entered = shadow_module("c", 1, 0);
    let mut world = colour_world(left.clone());
    let saber = give_with(&mut world, &left, "slot_saber", PARTY, None);
    let mut state = world.script_state().clone();
    let mut held = state.items.get_mut(&saber).expect("slot_saber");
    held.fitted = vec![String::new(), "shared_part".into()];
    drop(held);
    state.carry_known(&left, &middle);
    state.carry_known(&middle, &entered);
    let wanted_a = own_part(&left, "shared_part", "a");
    let wanted_b = own_part(&middle, "shared_part", "b");
    let wanted_c = own_part(&entered, "shared_part", "c");
    let holds_a = state.known.get(&wanted_a.template) == Some(&wanted_a);
    let holds_b = state.known.get(&wanted_b.template) == Some(&wanted_b);
    // The three descriptions this swap is judged by, so that it is the
    // disagreement and not a fixture mistake that the old rule would have
    // refused: a asks level 1, b and c both ask level 0 as their own
    // templates, and the new hilt takes 1.
    let asks = wanted_a.upgrade == Some((0, 1))
        && wanted_a.upgrade_level == 1
        && wanted_b.upgrade == Some((0, 1))
        && wanted_b.upgrade_level == 0
        && wanted_c.upgrade == Some((0, 1))
        && wanted_c.upgrade_level == 0
        && wanted_c.template == crate::inventory::own_template("shared_part", "c")
        && entered.get("shared_part") == Some(&wanted_c)
        && descriptions_of(&wanted_a, &wanted_b, "shared_part", false)
        && hilt_could_take(&left, "slot_saber", 1, &wanted_a)
        && hilt_level(&entered, "slot_saber_red") == 1;
    let new = state
        .make_item(&entered, "new_crystal", PARTY, 1, None)
        .expect("the crystal for the swap")
        .0;
    world.set_script_state(state.clone());
    let next = state.next_item;
    let before = state.clone();
    let mut out = seeded_out();
    let before_out = out.clone();
    let result = state.fit(&entered, &mut out, saber, new, None);
    world.set_script_state(state);
    let expected = vec![
        carried(
            saber,
            "slot_saber_red",
            1,
            0,
            vec![String::new(), "shared_part".to_string()],
        ),
        carried(
            crate::inventory::FIRST_ITEM + next,
            "old_crystal",
            1,
            0,
            Vec::new(),
        ),
    ];
    let goods = party_goods(&world);
    // The carried facts still travel, and the swap is still the colour swap
    // the probe means: one old colour mapping, and the crystal in hand
    // recolouring the hilt into the intended new one.
    let swap = swap_reaches_preserves(&entered, &before, saber, new, "slot_saber_red");
    let named = fitted_names(&before, saber, 1, "shared_part");
    let slots = both_slots_take(&entered, "slot_saber", "slot_saber_red", 1, 1);
    let reaches = swap && named && slots;
    let parts = swap_outcome(
        &world,
        &before,
        (&out, &before_out),
        &expected,
        next,
        "new_crystal",
        2,
    );
    let exact = holds_a && holds_b && asks && reaches && parts.iter().all(|(_, ok)| *ok);
    checks.push((
        "a colour swap goes ahead when every description of the fitted part allows it",
        check(
            result.is_ok() && exact,
            format!(
                "holds a {holds_a}, holds b {holds_b}, a asks {}, b asks {}, c asks {}, outcome {parts:?}, result {result:?}, goods {goods:?}",
                wanted_a.upgrade_level,
                wanted_b.upgrade_level,
                wanted_c.upgrade_level
            ),
        ),
    ));

    // The other direction of the same disagreement: the module entered reads
    // the fitted resref at a higher level than the module the part was
    // fitted in did, and the new hilt takes less than that. The engine
    // cannot tell which reading describes the part in hand, so it refuses.
    let left = shadow_module("a", 1, 0);
    let entered = shadow_module("b", 1, 3);
    let mut world = colour_world(left.clone());
    let saber = give_with(&mut world, &left, "slot_saber", PARTY, None);
    let mut state = world.script_state().clone();
    let mut held = state.items.get_mut(&saber).expect("slot_saber");
    held.fitted = vec![String::new(), "shared_part".into()];
    drop(held);
    state.carry_known(&left, &entered);
    let wanted_a = own_part(&left, "shared_part", "a");
    let wanted_b = own_part(&entered, "shared_part", "b");
    let holds_a = state.known.get(&wanted_a.template) == Some(&wanted_a);
    // The bare resref the save slot holds resolves to module b's own
    // template, and module b's reading is the only one above what the new
    // hilt takes: a asks level 0, b asks 3, the hilt takes 1.
    let reads_higher = entered
        .get("shared_part")
        .is_some_and(|facts| *facts == wanted_b)
        && wanted_a.upgrade == Some((0, 1))
        && wanted_a.upgrade_level == 0
        && wanted_b.upgrade == Some((0, 1))
        && wanted_b.upgrade_level == 3
        && descriptions_of(&wanted_a, &wanted_b, "shared_part", false)
        && hilt_could_take(&left, "slot_saber", 1, &wanted_a)
        && hilt_level(&entered, "slot_saber_red") == 1;
    let new = state
        .make_item(&entered, "new_crystal", PARTY, 1, None)
        .expect("the crystal for the swap")
        .0;
    world.set_script_state(state.clone());
    let before = state.clone();
    let mut out = seeded_out();
    let before_out = out.clone();
    let result = state.fit(&entered, &mut out, saber, new, None);
    let kept = state == before && out_same(&out, &before_out);
    world.set_script_state(state);
    let swapped = nothing_swapped(&world, "new_crystal", "old_crystal");
    let swap = swap_reaches_preserves(&entered, &before, saber, new, "slot_saber_red");
    let named = fitted_names(&before, saber, 1, "shared_part");
    let slots = both_slots_take(&entered, "slot_saber", "slot_saber_red", 1, 1);
    let reaches = swap && named && slots;
    checks.push((
        "a colour swap refuses when the module entered reads the fitted part higher than the new hilt",
        check(
            holds_a
                && reads_higher
                && reaches
                && result == Err(Notice::CannotUpgrade)
                && kept
                && swapped,
            format!(
                "holds a {holds_a}, reads higher {reads_higher}, swap {swap}, the slot names the part {named}, both hilts take the kind there {slots}, result {result:?}, kept {kept}, nothing swapped {swapped}"
            ),
        ),
    ));
    checks
}

fn output_rollback_checks() -> Checks {
    let mut checks: Checks = Vec::new();

    let catalog = missing_upgrade_catalog();
    let mut world = colour_world(catalog.clone());
    let player = world
        .snapshot()
        .body(Seat(0))
        .map_or(0, |entity| entity.id.0);
    let saber = give_with(
        &mut world,
        &catalog,
        "saber",
        player,
        Some(slots::RIGHT_WEAPON),
    );
    let mut state = world.script_state().clone();
    let mut held = state.items.get_mut(&saber).expect("saber");
    held.upgrades = crate::inventory::upgrade_bit(1).expect("row 1");
    drop(held);
    world.set_script_state(state.clone());
    let before = state.clone();
    let mut out = seeded_out();
    let before_out = out.clone();
    let result = state.unfit(&catalog, &mut out, saber, 1);
    let kept = state == before && out_same(&out, &before_out);
    world.set_script_state(state);
    checks.push((
        "a refused direct upgrade preserves the equipped item and every earlier output field",
        check(
            result.is_err() && kept,
            format!("result {result:?}, kept {kept}"),
        ),
    ));

    let catalog = colour_catalog();
    let mut world = colour_world(catalog.clone());
    let player = world
        .snapshot()
        .body(Seat(0))
        .map_or(0, |entity| entity.id.0);
    let saber = give_with(
        &mut world,
        &catalog,
        "saber",
        player,
        Some(slots::RIGHT_WEAPON),
    );
    let new = give_with(&mut world, &catalog, "new_crystal", PARTY, None);
    let mut state = world.script_state().clone();
    let before = state.clone();
    let mut out = seeded_out();
    let before_out = out.clone();
    let result = state.assemble(
        &catalog,
        &mut out,
        (
            saber,
            &[
                Fitting::In {
                    upgrade: new,
                    replace: None,
                },
                Fitting::Out(1),
            ],
        ),
        None,
    );
    let kept = state == before && out_same(&out, &before_out);
    world.set_script_state(state);
    checks.push((
        "a partial Assemble restores every item and every earlier output field",
        check(
            result.is_err() && kept,
            format!("result {result:?}, kept {kept}"),
        ),
    ));

    let catalog = colour_catalog();
    let mut world = colour_world(catalog.clone());
    let saber = give_with(&mut world, &catalog, "saber", PARTY, None);
    let new = give_with(&mut world, &catalog, "new_crystal", PARTY, None);
    let mut state = world.script_state().clone();
    let mut held = state.items.get_mut(&saber).expect("saber");
    held.stack = 2;
    held.upgrades = crate::inventory::upgrade_bit(1).expect("row 1");
    drop(held);
    world.set_script_state(state.clone());
    let before = state.clone();
    let mut out = seeded_out();
    let before_out = out.clone();
    let result = state.fit(&catalog, &mut out, saber, new, None);
    let kept = state == before && out_same(&out, &before_out);
    world.set_script_state(state);
    checks.push((
        "a refused colour swap on a stacked host rolls the split back as well",
        check(
            result.is_err() && kept,
            format!("result {result:?}, kept {kept}"),
        ),
    ));
    checks
}

pub fn return_atomic_probes() -> Checks {
    return_atomic_checks()
}

pub fn colour_refusal_probes() -> Checks {
    colour_refusal_checks()
}

pub fn output_rollback_probes() -> Checks {
    output_rollback_checks()
}

/// One step, seat 0 doing `actions`: what it was told.
fn step(world: &mut SimWorld, actions: &[Action]) -> Vec<Notice> {
    let mut input = TickInput::default();
    input.insert(Seat(0), Command::default());
    for action in actions {
        input.act(Seat(0), action.clone());
    }
    let tick = world.tick() + 1;
    crate::step(world, tick, &input, 8, StepReason::Authority);
    world
        .take_notices()
        .into_iter()
        .map(|(_, notice)| notice)
        .collect()
}

/// Seat 0's `action` stepped: the components after, and what it was told.
fn act(world: &mut SimWorld, action: Action) -> (i32, Vec<Notice>) {
    let told = step(world, &[action]);
    (world.script_state().components, told)
}

/// `script` run, then stepped: what was told, and custom token `n`.
fn run(world: &mut SimWorld, script: &str, n: i32) -> (Vec<Notice>, Option<String>) {
    let mut state = world.script_state().clone();
    state
        .pending
        .push(Pending::new(script, crate::script::MODULE));
    world.set_script_state(state);
    let told = step(world, &[]);
    let token = world.snapshot().talk.tokens.get(&n).cloned();
    (told, token)
}

/// Items of `template` anywhere.
fn count(world: &SimWorld, template: &str) -> usize {
    world
        .script_state()
        .items
        .values()
        .filter(|held| held.template == template)
        .count()
}

/// `template` made for `holder` (worn in `slot`) by hand: its id.
fn give(world: &mut SimWorld, template: &str, holder: u32, slot: Option<u8>) -> u32 {
    let mut state = world.script_state().clone();
    let made = state
        .make_item(&catalog(), template, holder, 1, slot)
        .map_or(0, |id| id.0);
    world.set_script_state(state);
    made
}

pub fn probes() -> Checks {
    let mut world = world();
    let me = world
        .snapshot()
        .body(Seat(0))
        .map_or(0, |entity| entity.id.0);
    world.set_player_skill(usize::from(workshop::REPAIR), 10);
    world.set_player_skill(usize::from(workshop::TREAT_INJURY), 10);
    let mut made = |template: &str| give(&mut world, template, PARTY, None);
    let (blaster, idol, rag, fitted) = (made("blaster"), made("idol"), made("rag"), made("rifle"));
    let (medkit, hkpart, lab_blaster) = (made("medkit"), made("hkpart"), made("pistol"));
    let gloves = made("gloves");
    let crate_box = 0x7d00_0001;
    give(&mut world, "compont", crate_box, None);
    let mut state = world.script_state().clone();
    if let Some(mut held) = state.items.get_mut(&fitted) {
        held.fitted = vec!["part".into()];
    }
    if let Some(id) = state.carried(crate_box).first()
        && let Some(mut held) = state.items.get_mut(&id.0)
    {
        held.stack = 7;
    }
    world.set_script_state(state);
    let bench = Workbench {
        list: String::new(),
        lab: false,
        maker: me,
    };
    let lab = Workbench {
        lab: true,
        list: workshop::LAB.into(),
        ..bench.clone()
    };
    let breakdown = |bench: &Workbench, item| Action::Breakdown {
        bench: bench.clone(),
        item,
    };
    let create = |bench: &Workbench, row| Action::Create {
        bench: bench.clone(),
        row,
    };
    let mut checks: Checks = Vec::new();

    let (after, told) = act(&mut world, breakdown(&bench, blaster));
    let gone = !world.script_state().items.contains_key(&blaster);
    checks.push((
        "a breakdown: the price (400 / 20) by Repair 10 of 20, the item gone, the bench told, its sound",
        check(
            after == 10
                && gone
                && told.iter().any(|notice| {
                    matches!(notice, Notice::Show(Screen::Workbench(bench)) if bench.stock == 10)
                })
                && told.contains(&sound(BROKEN)),
            format!("{after} components, gone {gone}, told {told:?}"),
        ),
    ));
    let (after, told) = act(&mut world, create(&bench, 0));
    let parts = world
        .script_state()
        .items
        .values()
        .filter(|held| held.template == "part" && held.holder == PARTY)
        .count();
    checks.push((
        "made: its price (200 / 20) taken, one to the party, its sound",
        check(
            after == 0 && parts == 1 && told.contains(&sound(MADE)),
            format!("{after} components, {parts} parts, told {told:?}"),
        ),
    ));
    let (after, told) = act(&mut world, create(&bench, 0));
    checks.push((
        "too few components: refused with the game's line",
        check(
            after == 0 && told.contains(&Notice::Line(116_297)),
            format!("{after}, {told:?}"),
        ),
    ));
    let refusals: Vec<Vec<Notice>> = [idol, rag, fitted]
        .into_iter()
        .map(|item| act(&mut world, breakdown(&bench, item)).1)
        .collect();
    checks.push((
        "not broken down: a plot item, one worth nothing, one with upgrades fitted",
        check(
            refusals
                == vec![
                    vec![Notice::CannotUse],
                    vec![Notice::CannotUse],
                    vec![Notice::Line(124_119)],
                ],
            format!("{refusals:?}"),
        ),
    ));
    let mut state = world.script_state().clone();
    state.components = 1000;
    world.set_script_state(state);
    let rifle = act(&mut world, create(&bench, 1)).1;
    let armour = act(&mut world, create(&bench, 2)).1;
    checks.push((
        "a recipe above the rank refused with the game's line, one 8 above it unlisted",
        check(
            rifle.contains(&Notice::Line(116_292)) && armour.contains(&Notice::CannotUse),
            format!("{rifle:?}, {armour:?}"),
        ),
    ));
    step(
        &mut world,
        &[Action::Equip {
            item: gloves,
            slot: slots::HANDS,
        }],
    );
    let (after, told) = act(&mut world, create(&bench, 1));
    checks.push((
        "the rank with what the maker wears: Repair 10 and gloves of +2 make the rifle (12)",
        check(
            after == 1000 - 200 && told.contains(&sound(MADE)),
            format!("{after} components, told {told:?}"),
        ),
    ));

    let refused: Vec<Vec<Notice>> = [lab_blaster, hkpart]
        .into_iter()
        .map(|item| step(&mut world, &[breakdown(&lab, item)]))
        .collect();
    step(&mut world, &[breakdown(&lab, medkit)]);
    let chemicals = world.script_state().chemicals;
    let short = step(&mut world, &[create(&lab, 0)]);
    checks.push((
        "the lab station: a pistol and an HK part refused, a medkit by Treat Injury 10 and Wisdom's +1 (20 × 11 / 20), too few chemicals its own line",
        check(
            refused == vec![vec![Notice::CannotUse]; 2]
                && chemicals == 11
                && short.contains(&Notice::Line(116_298)),
            format!("refused {refused:?}, {chemicals} chemicals, then {short:?}"),
        ),
    ));

    let library = world.level().scripts().cloned();
    let mut state = world.script_state().clone();
    let before = state.components;
    let took = library.map(|library| state.take_rest(&library, crate_box));
    let counted = state.components - before;
    world.set_script_state(state);
    let kept = count(&world, "compont");
    checks.push((
        "components taken are the party's count, no item",
        check(
            took.is_some_and(|(any, gained)| any && gained.components == 7)
                && counted == 7
                && kept == 0,
            format!("took {took:?}, +{counted} components, items kept {kept}"),
        ),
    ));

    let before = world.script_state().components;
    let (told, valid) = run(&mut world, "probe_pool", 1);
    let gained = world.script_state().components - before;
    let line = Notice::Count {
        line: ACQUIRED_COMPONENTS,
        count: 5,
    };
    checks.push((
        "made on the player: the party's count (+5), no item made, the game's line",
        check(
            gained == 5
                && valid.as_deref() == Some("0")
                && count(&world, "compont") == 0
                && told.contains(&line),
            format!("+{gained}, an item {valid:?}, told {told:?}"),
        ),
    ));
    let before = world.script_state().components;
    let (_, valid) = run(&mut world, "probe_hoard", 2);
    step(&mut world, &[]);
    let state = world.script_state();
    let npc = state
        .party
        .available
        .get(&(NPC as u8))
        .and_then(|npc| npc.object);
    let hoard = npc.and_then(|npc| state.hoards.get(&npc).copied());
    let untouched = state.components == before;
    checks.push((
        "a creature outside the party: its starting two and three made on it are its own count, no item",
        check(
            hoard == Some((5, 0))
                && untouched
                && valid.as_deref() == Some("0")
                && count(&world, "compont") == 0,
            format!("its count {hoard:?}, the party's untouched {untouched}, an item {valid:?}"),
        ),
    ));
    run(&mut world, "probe_join", 0);
    let state = world.script_state();
    let joined = state.components - before;
    let kept = npc.and_then(|npc| state.hoards.get(&npc).copied());
    checks.push((
        "it joins: its count added to the party's, its own kept (Q-0232)",
        check(
            joined == 5 && kept == Some((5, 0)),
            format!("+{joined}, its own {kept:?}"),
        ),
    ));
    let holders = |template: &str| -> Vec<(u32, Option<u8>)> {
        state
            .items
            .values()
            .filter(|held| held.template == template)
            .map(|held| (held.holder, held.slot))
            .collect()
    };
    let (blade, gloves) = (holders("vibroblade"), holders("gloves"));
    checks.push((
        "it joins: the blade it carries the party's, the gloves it wears its own (TransferInventory)",
        check(
            blade == [(PARTY, None)]
                && npc.is_some_and(|npc| gloves.contains(&(npc, Some(slots::HANDS)))),
            format!("its blade with {blade:?}, its gloves with {gloves:?}"),
        ),
    ));

    checks.extend(assembled(&mut world, me));
    checks
}

/// The upgrade bench's Assemble on a rifle the party carries and a
/// lightsaber the player wears.
fn assembled(world: &mut SimWorld, me: u32) -> Checks {
    let mut made = |template: &str| give(world, template, PARTY, None);
    let (rifle, scope, scope2, pack, crystal, lens) = (
        made("scoped"),
        made("scope"),
        made("scope2"),
        made("pack"),
        made("crystal"),
        made("lens"),
    );
    let saber = give(world, "saber", me, Some(slots::RIGHT_WEAPON));
    let assemble = |item, changes: &[Fitting]| Action::Assemble {
        item,
        changes: changes.to_vec(),
    };
    let fit = |upgrade| Fitting::In {
        upgrade,
        replace: None,
    };
    let fitted = |world: &SimWorld, item: u32| {
        world
            .script_state()
            .items
            .get(&item)
            .map_or_else(Vec::new, |held| held.fitted.clone())
    };
    let mut checks: Checks = Vec::new();
    let told = step(world, &[assemble(rifle, &[fit(pack), fit(scope2)])]);
    let unchanged = fitted(world, rifle).iter().all(String::is_empty)
        && world.script_state().items.contains_key(&pack);
    checks.push((
        "a level-2 scope in a level-1 rifle refuses the whole visit: the pack fitted with it is not",
        check(
            told.contains(&Notice::CannotUpgrade) && unchanged,
            format!("told {told:?}, unchanged {unchanged}"),
        ),
    ));
    step(world, &[assemble(rifle, &[fit(scope), fit(pack)])]);
    let both = fitted(world, rifle);
    let used = [scope, pack]
        .iter()
        .all(|part| !world.script_state().items.contains_key(part));
    checks.push((
        "Assemble makes its changes together: scope and pack fitted, taken from the party",
        check(
            both == ["scope", "pack", ""] && used,
            format!("fitted {both:?}, taken {used}"),
        ),
    ));
    step(world, &[assemble(saber, &[fit(crystal)])]);
    let worn = world
        .script_state()
        .items
        .get(&saber)
        .is_some_and(|held| held.holder == me && held.slot.is_some());
    let crystals = fitted(world, saber);
    checks.push((
        "a crystal of level 3 fits a level-1 lightsaber, which stays worn",
        check(
            crystals.first().map(String::as_str) == Some("crystal") && worn,
            format!("fitted {crystals:?}, worn {worn}"),
        ),
    ));
    step(world, &[assemble(saber, &[fit(lens)])]);
    let held = world.script_state().items.get(&saber).cloned();
    let packed = held
        .as_ref()
        .is_some_and(|held| held.holder == PARTY && held.slot.is_none());
    checks.push((
        "a lens only Bastila may wear: the lightsaber, off its wearer on the bench, goes to the pack",
        check(
            packed && held.as_ref().is_some_and(|held| held.fitted[3] == "lens"),
            format!("{held:?}"),
        ),
    ));
    checks.extend(stacks_upgraded(world));
    checks
}

/// The survivor pistols' stacks the party carries: each stack's count and
/// the parts in its slots, sorted.
fn survivors(world: &SimWorld) -> Vec<(u16, Vec<String>)> {
    let mut stacks: Vec<(u16, Vec<String>)> = world
        .script_state()
        .items
        .values()
        .filter(|held| held.template == "survivor" && held.holder == PARTY)
        .map(|held| {
            let parts = held.fitted.iter().filter(|part| !part.is_empty());
            (held.stack, parts.cloned().collect())
        })
        .collect();
    stacks.sort();
    stacks
}

/// Upgrading one of a stack of pistols at the bench, and taking the
/// upgraded ones out of a container: an item stacks only with items of the
/// same upgrades.
fn stacks_upgraded(world: &mut SimWorld) -> Checks {
    let pistols = [0; 4].map(|_| give(world, "survivor", PARTY, None))[0];
    let scopes = [0; 2].map(|_| give(world, "scope", PARTY, None));
    let packs = give(world, "pack", PARTY, None);
    let fit = |item, upgrade| Action::Assemble {
        item,
        changes: vec![Fitting::In {
            upgrade,
            replace: None,
        }],
    };
    let with =
        |count: u16, parts: &[&str]| (count, parts.iter().map(|part| part.to_string()).collect());
    let mut checks: Checks = Vec::new();
    let mut expect =
        |world: &mut SimWorld, what, actions: &[Action], want: Vec<(u16, Vec<String>)>| {
            step(world, actions);
            let now = survivors(world);
            checks.push((what, check(now == want, format!("{now:?}"))));
        };
    expect(
        world,
        "one of a stack of four upgraded at the bench: split off, three plain left",
        &[fit(pistols, scopes[0])],
        vec![with(1, &["scope"]), with(3, &[])],
    );
    expect(
        world,
        "a second one upgraded the same: joins the first upgraded one",
        &[fit(pistols, scopes[1])],
        vec![with(2, &[]), with(2, &["scope"])],
    );
    expect(
        world,
        "a third one upgraded differently: a stack of its own",
        &[fit(pistols, packs)],
        vec![with(1, &[]), with(1, &["pack"]), with(2, &["scope"])],
    );
    let scoped = world
        .script_state()
        .items
        .iter()
        .find(|(_, held)| held.template == "survivor" && held.stack == 2)
        .map_or(0, |(id, _)| *id);
    let crate_box = 0x7d00_0001;
    let library = world.level().scripts().cloned();
    let mut state = world.script_state().clone();
    let put = state.put_in(scoped, crate_box, None);
    if let Some(library) = library {
        state.take_item(&library, crate_box, put, None);
    }
    world.set_script_state(state);
    expect(
        world,
        "the upgraded ones put in a container and taken back: their upgrades kept, their own stack",
        &[],
        vec![with(1, &[]), with(1, &["pack"]), with(2, &["scope"])],
    );
    checks
}
