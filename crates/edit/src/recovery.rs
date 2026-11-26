//! Read-only ownership accounting for a save (REC-01).
//!
//! This counts what a save actually holds once, without writing, renaming or
//! listing the saves folder. It separates party-loose items, items carried by
//! a character, equipped items, items held by an unknown holder, and fitted
//! upgrade parts. A template whose facts the catalog does not have remains
//! unresolved.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use sim::inventory::{COLOUR_SLOT, Catalog, Item, PARTY};
use sim::{Save, SimId};

/// How one item is held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holding {
    /// The party carries it loose.
    PartyLoose,
    /// A character carries it without wearing it.
    Carried,
    /// A character wears it in a slot.
    Equipped,
    /// A holder the save's characters do not name.
    Other,
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
    /// KOTOR's fitted upgrade rows, as bits set.
    pub k1_rows: usize,
    /// TSL's fitted upgrade slots whose template is not empty.
    pub k2_slots: usize,
    /// A TSL colour crystal fitted in a lightsaber's colour slot.
    pub colour: bool,
    pub catalog_known: bool,
}

/// What a save holds, counted once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnershipReport {
    pub items: Vec<ItemOwnership>,
    pub units: u64,
    pub party_loose: usize,
    pub carried: usize,
    pub equipped: usize,
    pub other: usize,
    pub k1_rows: usize,
    pub k2_slots: usize,
    pub colours: usize,
    pub catalog_unknown: usize,
    pub unknown_holders: Vec<u32>,
    pub ambiguities: Vec<String>,
}

/// The item's slot kind, if the catalog knows its template.
fn colour_slot(catalog: Option<&Catalog>, template: &str, fitted: &[String]) -> bool {
    let Some(facts) = catalog.and_then(|catalog| catalog.get(template)) else {
        return false;
    };
    let Some(at) = facts
        .slot_kinds
        .iter()
        .position(|kind| *kind == COLOUR_SLOT)
    else {
        return false;
    };
    fitted.get(at).is_some_and(|part| !part.is_empty())
}

/// What `save` holds. This reads only `save` and `catalog`; it does not write
/// files or change the save.
pub fn inspect(save: &Save, catalog: Option<&Catalog>) -> OwnershipReport {
    let known_holders: HashSet<u32> = crate::characters(save, None)
        .iter()
        .filter_map(|character| crate::holder_of(save, &character.who))
        .collect();
    let mut report = OwnershipReport {
        items: Vec::new(),
        units: 0,
        party_loose: 0,
        carried: 0,
        equipped: 0,
        other: 0,
        k1_rows: 0,
        k2_slots: 0,
        colours: 0,
        catalog_unknown: 0,
        unknown_holders: Vec::new(),
        ambiguities: Vec::new(),
    };
    let mut slots: BTreeMap<(u32, u8), usize> = BTreeMap::new();
    let mut unknown = BTreeSet::new();
    for (id, item) in save.script.items.iter() {
        let holding = match item.slot {
            Some(_) => Holding::Equipped,
            None if item.holder == PARTY => Holding::PartyLoose,
            None if known_holders.contains(&item.holder) => Holding::Carried,
            None => Holding::Other,
        };
        if let Some(slot) = item.slot {
            *slots.entry((item.holder, slot)).or_default() += 1;
        }
        if holding == Holding::Other {
            unknown.insert(item.holder);
        }
        let k1_rows = item.upgrades.count_ones() as usize;
        let k2_slots = item.fitted.iter().filter(|part| !part.is_empty()).count();
        let known = catalog
            .and_then(|catalog| catalog.get(&item.template))
            .is_some();
        if !known {
            report.catalog_unknown += 1;
        }
        let colour = colour_slot(catalog, &item.template, &item.fitted);
        if k1_rows != 0 && k2_slots != 0 {
            report.ambiguities.push(format!(
                "item {id} holds both KOTOR upgrade rows and TSL fitted slots"
            ));
        }
        if item.stack == 0 {
            report
                .ambiguities
                .push(format!("item {id} has a zero stack"));
        }
        report.units += u64::from(item.stack);
        report.k1_rows += k1_rows;
        report.k2_slots += k2_slots;
        if colour {
            report.colours += 1;
        }
        match holding {
            Holding::PartyLoose => report.party_loose += 1,
            Holding::Carried => report.carried += 1,
            Holding::Equipped => report.equipped += 1,
            Holding::Other => report.other += 1,
        }
        report.items.push(ItemOwnership {
            id: *id,
            template: item.template.clone(),
            holder: item.holder,
            slot: item.slot,
            stack: item.stack,
            holding,
            k1_rows,
            k2_slots,
            colour,
            catalog_known: known,
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
pub fn item(save: &Save, id: u32) -> Option<Item> {
    save.script
        .items
        .get(&id)
        .map(|item| Item { ..item.clone() })
}

/// A save's character holder by seat, for a bounded fixture.
pub fn seat_holder(save: &Save, seat: sim::Seat) -> Option<u32> {
    save.snapshot
        .entities
        .iter()
        .find(|entity| entity.kind == sim::Kind::Avatar(seat))
        .map(|entity| entity.id.0)
}

/// The player's body id, if the save has a first seat.
pub fn first_seat_holder(save: &Save) -> Option<SimId> {
    seat_holder(save, sim::Seat(0)).map(SimId)
}
