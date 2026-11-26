//! Read-only ownership accounting for a save (REC-01).
//!
//! This counts what a save actually holds once, without writing, renaming or
//! listing the saves folder. It says who holds each item, where the facts of
//! its template came from, and which items the save cannot explain.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use sim::inventory::{COLOUR_SLOT, Catalog, Item, ItemFacts, PARTY, facts_in_play};
use sim::module::ObjectKind;
use sim::{Kind, Save, SimId};

/// Who holds one item, as the save says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holding {
    /// The party carries it loose (`PARTY` is its holder).
    PartyLoose,
    /// A seat's player, a companion or one away carries it unworn.
    Carried,
    /// A character wears it in a slot (`Item::slot` names the slot).
    Equipped,
    /// It lies on the area's floor: the area holds it (`AREA`) and the save
    /// lists it among the things lying there (`State::floor`).
    Floor,
    /// One of the area's containers holds it (`ObjectKind::Placeable`).
    Container,
    /// It is a store's stock (`ObjectKind::Store`).
    Store,
    /// An area creature the save names no character for holds it.
    Creature,
    /// An object of the area that is not a holder the engine gives items to:
    /// a door, trigger, waypoint, sound or encounter. The save names the
    /// holder; what it holds there is the anomaly.
    AreaObject,
    /// Nothing in the save answers for `holder`: no character, no floor
    /// entry and no object of the area.
    Unknown,
}

/// Where the facts of an item's template were found, and whether they were
/// looked for at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Facts {
    /// The module's catalog has the template.
    Catalog,
    /// The save carries the facts of a template its module lacks
    /// (`State::known`).
    Carried,
    /// Neither has it, so the item's template is unresolved.
    Absent,
    /// No catalog was given, so nothing was checked and nothing is claimed.
    NotChecked,
}

/// One item in the save, counted once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemOwnership {
    pub id: u32,
    pub template: String,
    pub holder: u32,
    pub slot: Option<u8>,
    pub stack: u16,
    pub holding: Holding,
    pub facts: Facts,
    /// A store's stock, whose stack is a count the game never spends.
    pub infinite: bool,
    /// KOTOR's fitted upgrade rows, as bits set.
    pub kotor_rows: usize,
    /// TSL's fitted upgrade slots whose template is not empty.
    pub tsl_slots: usize,
    /// A TSL colour crystal fitted in a lightsaber's colour slot.
    pub colour: bool,
}

/// What a save holds, counted once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnershipReport {
    pub items: Vec<ItemOwnership>,
    /// The stacks that are not a store's stock, summed over every holder.
    pub units: u64,
    pub party_loose: usize,
    pub carried: usize,
    pub equipped: usize,
    pub floor: usize,
    pub containers: usize,
    pub store: usize,
    pub creatures: usize,
    pub area_objects: usize,
    pub unknown: usize,
    /// Items whose stack is never spent (a store's stock).
    pub infinite: usize,
    pub kotor_rows: usize,
    pub tsl_slots: usize,
    pub colours: usize,
    pub facts_carried: usize,
    pub facts_absent: usize,
    pub unknown_holders: Vec<u32>,
    pub ambiguities: Vec<String>,
}

/// Whether the facts say this item wears a colour crystal in a colour slot.
fn colour_slot(facts: Option<&ItemFacts>, fitted: &[String]) -> bool {
    let Some(at) = facts.and_then(|facts| {
        facts
            .slot_kinds
            .iter()
            .position(|kind| *kind == COLOUR_SLOT)
    }) else {
        return false;
    };
    fitted.get(at).is_some_and(|part| !part.is_empty())
}

/// What kind of area object `holder` is, if the save has such an object.
fn object_kind(save: &Save, holder: u32) -> Option<ObjectKind> {
    save.snapshot
        .get(SimId(holder))
        .and_then(|entity| match entity.kind {
            Kind::Object(kind, _) => Some(kind),
            _ => None,
        })
}

/// The bodies the save's characters have: every seat's player, and every
/// companion the party lists, whether or not the save also carries that
/// companion's sheet (`edit::characters` only reports the sheeted ones).
fn character_holders(save: &Save) -> HashSet<u32> {
    let mut ids: HashSet<u32> = save
        .snapshot
        .entities
        .iter()
        .filter(|entity| matches!(entity.kind, Kind::Avatar(_)))
        .map(|entity| entity.id.0)
        .collect();
    for slot in save.script.party.available.values() {
        if let Some(object) = slot.object {
            ids.insert(object);
        }
        if let Some(kept) = &slot.kept {
            ids.insert(kept.from);
        }
    }
    ids
}

/// Who holds `item`: the party, a character, the area's floor, one of the
/// area's objects, or nothing the save names. An item worn in a slot by a
/// holder nothing answers for is unknown too, not equipped. A save can keep
/// listing an item among those lying on the floor after it was picked up, so
/// the floor class needs the area as holder as well as the listing.
fn holding(save: &Save, id: u32, item: &Item, known: &HashSet<u32>) -> Holding {
    if item.holder == PARTY {
        return Holding::PartyLoose;
    }
    if known.contains(&item.holder) {
        return if item.slot.is_some() {
            Holding::Equipped
        } else {
            Holding::Carried
        };
    }
    if save.script.floor.contains_key(&id) && item.holder == sim::script::AREA.0 {
        return Holding::Floor;
    }
    match object_kind(save, item.holder) {
        Some(ObjectKind::Store) => Holding::Store,
        Some(ObjectKind::Placeable) => Holding::Container,
        Some(ObjectKind::Creature) => Holding::Creature,
        Some(_) => Holding::AreaObject,
        None => Holding::Unknown,
    }
}

/// What `save` holds. This reads only `save` and `catalog`: it writes no
/// files and leaves the save byte-for-byte as it was.
pub fn inspect(save: &Save, catalog: Option<&Catalog>) -> OwnershipReport {
    let known_holders = character_holders(save);
    let mut report = OwnershipReport {
        items: Vec::new(),
        units: 0,
        party_loose: 0,
        carried: 0,
        equipped: 0,
        floor: 0,
        containers: 0,
        store: 0,
        creatures: 0,
        area_objects: 0,
        unknown: 0,
        infinite: 0,
        kotor_rows: 0,
        tsl_slots: 0,
        colours: 0,
        facts_carried: 0,
        facts_absent: 0,
        unknown_holders: Vec::new(),
        ambiguities: Vec::new(),
    };
    let mut slots: BTreeMap<(u32, u8), usize> = BTreeMap::new();
    let mut unknown = BTreeSet::new();
    for (id, item) in save.script.items.iter() {
        let holding = holding(save, *id, item, &known_holders);
        if let Some(slot) = item.slot {
            *slots.entry((item.holder, slot)).or_default() += 1;
        }
        if holding == Holding::Unknown {
            unknown.insert(item.holder);
        }
        let facts =
            catalog.and_then(|catalog| facts_in_play(catalog, &save.script.known, &item.template));
        let source = match (catalog, facts) {
            (None, _) => Facts::NotChecked,
            (Some(_), None) => Facts::Absent,
            (Some(catalog), Some(_)) => {
                if catalog.get(&item.template).is_some() {
                    Facts::Catalog
                } else {
                    Facts::Carried
                }
            }
        };
        let kotor_rows = item.upgrades.count_ones() as usize;
        let tsl_slots = item.fitted.iter().filter(|part| !part.is_empty()).count();
        let colour = colour_slot(facts, &item.fitted);
        if kotor_rows != 0 && tsl_slots != 0 {
            report.ambiguities.push(format!(
                "item {id} holds both KOTOR upgrade rows and TSL fitted slots"
            ));
        }
        if item.stack == 0 && !item.infinite {
            report
                .ambiguities
                .push(format!("item {id} has a zero stack"));
        }
        if item.infinite {
            report.infinite += 1;
        } else {
            report.units += u64::from(item.stack);
        }
        report.kotor_rows += kotor_rows;
        report.tsl_slots += tsl_slots;
        if colour {
            report.colours += 1;
        }
        match holding {
            Holding::PartyLoose => report.party_loose += 1,
            Holding::Carried => report.carried += 1,
            Holding::Equipped => report.equipped += 1,
            Holding::Floor => report.floor += 1,
            Holding::Container => report.containers += 1,
            Holding::Store => report.store += 1,
            Holding::Creature => report.creatures += 1,
            Holding::AreaObject => {
                report.area_objects += 1;
                report.ambiguities.push(format!(
                    "item {id} is held by object {}, which does not hold items",
                    item.holder
                ));
            }
            Holding::Unknown => report.unknown += 1,
        }
        match source {
            Facts::Carried => report.facts_carried += 1,
            Facts::Absent => report.facts_absent += 1,
            Facts::Catalog | Facts::NotChecked => {}
        }
        report.items.push(ItemOwnership {
            id: *id,
            template: item.template.clone(),
            holder: item.holder,
            slot: item.slot,
            stack: item.stack,
            holding,
            facts: source,
            infinite: item.infinite,
            kotor_rows,
            tsl_slots,
            colour,
        });
    }
    for ((holder, slot), count) in slots {
        if count > 1 {
            report.ambiguities.push(format!(
                "holder {holder} has {count} items equipped in slot {slot}"
            ));
        }
    }
    report.unknown_holders = unknown.into_iter().collect();
    report.items.sort_by_key(|item| item.id);
    report
}

/// A save's item by id, for a bounded fixture.
pub fn held_item(save: &Save, id: u32) -> Option<Item> {
    save.script
        .items
        .get(&id)
        .map(|item| Item { ..item.clone() })
}

/// A save's character body by seat, as the holder of what that character
/// carries and wears.
pub fn seat_holder(save: &Save, seat: sim::Seat) -> Option<SimId> {
    save.snapshot
        .entities
        .iter()
        .find(|entity| entity.kind == Kind::Avatar(seat))
        .map(|entity| entity.id)
}
