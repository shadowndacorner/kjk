//! Read-only ownership accounting for a save (REC-01).
//!
//! This counts what a save actually holds once, without writing, renaming or
//! listing the saves folder. It says who holds each item, where the facts of
//! its template came from, whether the parts installed in it are described
//! anywhere, and which items the save cannot explain.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use sim::inventory::{COLOUR_SLOT, Catalog, Item, ItemFacts, PARTY, facts_in_play};
use sim::module::ObjectKind;
use sim::{EntityState, Kind, Save, SimId};

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
    /// entry and no object of the area, in this module or in the ones the
    /// save has already played.
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

/// How the save shows that an item carries a colour part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colour {
    /// A part sits in one of the item's colour slots, which is how a fitted
    /// colour crystal is saved.
    Fitted,
    /// Nothing in a colour slot, and this module's catalog has a colour
    /// crystal that names the item's template among the hilts it recolours.
    /// A colour swap changes the hilt's own template and fits nothing else, so
    /// this is how a recoloured hilt is saved. The catalog cannot say whether
    /// such a template is the colour a hilt took or the hilt a colour came off,
    /// so a swap between two hilts shows up on both.
    Template,
    /// Neither sign, with a catalog to ask.
    None,
    /// A catalog was given, but the item's own template is not described, so
    /// its colour slots cannot be found and nothing is claimed.
    Unknown,
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
    /// How many identical units this id stands for: an item that stacks joins
    /// units with the same fields, so its installed parts are there once per
    /// unit. A store's stock is one row of unsent stock, so it counts once.
    pub units: usize,
    /// KOTOR's fitted upgrade rows, as bits set, over every unit.
    pub kotor_rows: usize,
    /// TSL's fitted upgrade slots whose template is not empty, over every unit.
    pub tsl_slots: usize,
    /// Fitted parts this save cannot explain: nothing describes the part, the
    /// part is not an upgrade of the kind the host's slot takes, or the host's
    /// facts declare no such slot. Counted per id, not per unit.
    pub unresolved_parts: usize,
    pub unlisted_rows: usize,
    /// How the save shows a colour part on this item.
    pub colour: Colour,
}

/// What a save holds, counted once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnershipReport {
    pub items: Vec<ItemOwnership>,
    /// The sum of every finite stack, in every holder: a store's stock is left
    /// out because its stack is a count the game never spends, and everything
    /// else counts, including an item that has a zero stack.
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
    pub parts_unresolved: usize,
    pub rows_unlisted: usize,
    /// Items that carry a colour part: the ones with a part in a colour slot
    /// plus the ones whose own template a colour crystal recolours. Each item
    /// counts once, whichever sign it shows.
    pub colours: usize,
    pub colour_fitted: usize,
    pub colour_by_template: usize,
    pub colour_unknown: usize,
    pub facts_carried: usize,
    pub facts_absent: usize,
    /// Holder ids nothing in the save answers for. A holder the save does name,
    /// but which is not an object that holds items, is reported as an anomaly
    /// instead, because its id is known to be wrong rather than missing.
    pub unknown_holders: Vec<u32>,
    pub ambiguities: Vec<String>,
}

/// Every object the save names: this module's entities, and those of the
/// modules it has already played, which the save keeps in its stash. An item
/// can still be held by an object of a module the party left: the holder stays
/// in the item table while the body is in the stash.
fn entities(save: &Save) -> impl Iterator<Item = &EntityState> {
    save.snapshot.entities.iter().chain(
        save.snapshot
            .stash
            .iter()
            .flat_map(|left| left.entities.iter()),
    )
}

/// What kind of area object `holder` is, if the save has such an object. An id
/// names one object, in this module or in one already played.
fn object_kind(save: &Save, holder: u32) -> Option<ObjectKind> {
    entities(save)
        .find(|entity| entity.id.0 == holder)
        .and_then(|entity| match entity.kind {
            Kind::Object(kind, _) => Some(kind),
            _ => None,
        })
}

/// The bodies the save's characters have: every seat's player, and every
/// companion the party lists, whether or not the save also carries that
/// companion's sheet (`edit::characters` only reports the sheeted ones), in
/// this module or in one already played. A companion kept while away names its
/// body only through the party table's `kept.from`.
fn character_holders(save: &Save) -> HashSet<u32> {
    let mut ids: HashSet<u32> = entities(save)
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
    // TSL's puppets are party members with their own bodies; a puppet who has
    // joined wears and carries things under its own object id. A puppet that has
    // not joined is one of the area's creatures, and its holdings are counted as
    // a creature's, which is what the engine's own puppet listing does.
    for puppet in save.script.party.puppets.values() {
        if puppet.joined
            && let Some(object) = puppet.object
        {
            ids.insert(object);
        }
    }
    ids
}

/// Whether the save names an entity of any kind with this id, in this module
/// or in one already played.
fn entity_named(save: &Save, id: u32) -> bool {
    entities(save).any(|entity| entity.id.0 == id)
}

/// Who holds `item`: the party, a character, the area's floor, one of the
/// area's objects, or nothing the save names. An item worn in a slot by a
/// holder nothing answers for is unknown too, not equipped. A save can keep
/// listing an item among those lying on the floor after it was picked up, so
/// the floor class needs the area as holder as well as the listing; an item the
/// area holds that the save does not list is not on the floor either.
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

/// How the save shows a colour part on `item`. A part in a colour slot wins:
/// the template sign alone cannot tell a recoloured hilt from the hilt the
/// colour came off, so it is only reported when there is no part to read.
/// Whether `part` is a crystal this module can speak of a colour swap with: its
/// facts name a hilt, this module catalogues that hilt, and the hilt is of the
/// host's base item. A save that has already had the swap done names the host's
/// own template, and one that has it still to do names a sibling hilt; a crystal
/// that recolours a template this module does not have proves nothing about
/// colour, because no route here could have installed it.
fn recolours_a_catalogued_hilt(catalog: &Catalog, host: &ItemFacts, part: &ItemFacts) -> bool {
    part.recolours.iter().any(|made| {
        catalog
            .get(made)
            .is_some_and(|hilt| hilt.base_item == host.base_item)
    })
}

fn colour_sign(
    catalog: Option<&Catalog>,
    facts: Option<&ItemFacts>,
    known: &std::collections::BTreeMap<String, ItemFacts>,
    item: &Item,
) -> Colour {
    let Some(catalog) = catalog else {
        return Colour::NotChecked;
    };
    let Some(facts) = facts else {
        return Colour::Unknown;
    };
    let mut fitted = false;
    for (at, kind) in facts.slot_kinds.iter().enumerate() {
        if *kind != COLOUR_SLOT {
            continue;
        }
        let Some(part) = item.fitted.get(at) else {
            continue;
        };
        if part.is_empty() {
            continue;
        }
        fitted = true;
        // Only something that recolours a hilt is a colour part. A colour slot
        // holding anything else leaves the colour unsettled rather than settled
        // in favour of a swap.
        let colour = facts_in_play(catalog, known, part)
            .is_some_and(|part_facts| recolours_a_catalogued_hilt(catalog, facts, part_facts));
        if !colour {
            return Colour::Unknown;
        }
    }
    if fitted {
        return Colour::Fitted;
    }
    if catalog
        .iter()
        .any(|crystal| crystal.recolours.iter().any(|made| made == &item.template))
    {
        return Colour::Template;
    }
    Colour::None
}

/// How many identical units `item` stands for, and whether the count of its
/// installed parts can be multiplied by it at all. A store's stock is one row
/// of unsent stock; an item that claims no units at all is still one row of the
/// save's item table, and its installed parts are counted once (and the empty
/// stack is reported as an anomaly).
fn units(item: &Item) -> usize {
    if item.infinite {
        return 1;
    }
    usize::from(item.stack).max(1)
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
        parts_unresolved: 0,
        rows_unlisted: 0,
        colours: 0,
        colour_fitted: 0,
        colour_by_template: 0,
        colour_unknown: 0,
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
        if holding == Holding::Unknown && !entity_named(save, item.holder) {
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
        let rows = item.upgrades.count_ones() as usize;
        let fitted = item.fitted.iter().filter(|part| !part.is_empty()).count();
        // Each unit of a stack carries the same installed parts, so the count
        // of parts follows the stack; `State::one_of` splits one unit out to
        // edit it, which is how a stack of upgraded items comes about.
        let units = units(item);
        let (kotor_rows, tsl_slots) = match (rows.checked_mul(units), fitted.checked_mul(units)) {
            (Some(rows), Some(slots)) => (rows, slots),
            _ => {
                report.ambiguities.push(format!(
                    "item {id} holds more installed parts than can be counted"
                ));
                (rows, fitted)
            }
        };
        // The parts installed in the item have to be described somewhere too,
        // or the item is only half accounted for.
        let mut unresolved_parts = 0;
        if let Some(catalog) = catalog {
            for (slot, part) in item.fitted.iter().enumerate() {
                if part.is_empty() {
                    continue;
                }
                // The host has to declare the slot the part sits in, and the
                // part has to be an upgrade of the kind that slot takes;
                // otherwise the save claims a fitted part this save cannot
                // explain, which is counted as unresolved, not as fitted.
                let declared = facts.and_then(|facts| facts.slot_kinds.get(slot)).copied();
                let described = facts_in_play(catalog, &save.script.known, part);
                let upgrade = described.and_then(|part_facts| part_facts.upgrade);
                let host_level = facts.map(|facts| facts.upgrade_level);
                let part_level = described.map(|part_facts| part_facts.upgrade_level);
                // The part may also have come with the host's template instead
                // of being installed on the bench: an item made of a template
                // starts with the parts that template lists, whatever their
                // level, so the bench's level rule does not speak of those.
                let from_the_template = facts
                    .is_some_and(|facts| facts.parts.get(slot).map(String::as_str) == Some(part));
                // What the colour route installs in a colour slot is a crystal
                // that recolours another hilt this module catalogues, which is
                // what its own route test asks for.
                let colour_part = described.is_some_and(|part_facts| {
                    facts.is_some_and(|host| recolours_a_catalogued_hilt(catalog, host, part_facts))
                });
                let why = match (declared, described.is_some(), upgrade) {
                    (None, _, _) => Some(format!(
                        "item {id} has the part {part} in slot {slot}, which its host's facts do not declare"
                    )),
                    (_, false, _) => Some(format!(
                        "item {id} has the part {part} in slot {slot}, which neither the module nor the save describes"
                    )),
                    // A colour slot holds what the colour route installed: a
                    // crystal that recolours a hilt. The route changes the
                    // hilt's own template instead of fitting an upgrade, so
                    // such a part needs no upgrade of its own. Anything else
                    // there is a part this save cannot explain.
                    (Some(COLOUR_SLOT), true, None) if colour_part => None,
                    (Some(COLOUR_SLOT), true, None) => Some(format!(
                        "item {id} has the part {part} in colour slot {slot}, which is no crystal that recolours a hilt this module catalogues"
                    )),
                    (Some(_), true, None) => Some(format!(
                        "item {id} has the part {part} in slot {slot}, which is not an upgrade this module or the save describes"
                    )),
                    (Some(kind), true, Some((_, part_kind))) if part_kind != kind => Some(format!(
                        "item {id} has the part {part} in slot {slot}, whose kind {part_kind} is not the kind {kind} its host declares"
                    )),
                    // And the host has to be able to take the part: one of a
                    // level above the host's own could not have been fitted
                    // there, unless it is a crystal, which any hilt takes.
                    (Some(_), true, Some((_, part_kind)))
                        if part_kind != sim::inventory::CRYSTAL
                            && !from_the_template
                            && part_level.unwrap_or(0) > host_level.unwrap_or(0) =>
                    {
                        Some(format!(
                            "item {id} has the part {part} in slot {slot}, whose level {} is above the level {} its host takes",
                            part_level.unwrap_or(0),
                            host_level.unwrap_or(0)
                        ))
                    }
                    _ => None,
                };
                if let Some(why) = why {
                    unresolved_parts += 1;
                    report.ambiguities.push(why);
                }
            }
        }
        // And a KOTOR row the item says is fitted has to be a row the host's
        // facts list.
        let mut unlisted_rows = 0;
        if let Some(facts) = facts {
            for row in 0..32 {
                if item.upgrades & (1u32 << row) == 0 {
                    continue;
                }
                if facts.takes(row as u8).is_none() {
                    unlisted_rows += 1;
                    report.ambiguities.push(format!(
                        "item {id} carries KOTOR upgrade row {row}, which its host's facts do not list"
                    ));
                }
            }
        }
        let colour = colour_sign(catalog, facts, &save.script.known, item);
        if rows != 0 && fitted != 0 {
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
        report.parts_unresolved += unresolved_parts;
        report.rows_unlisted += unlisted_rows;
        match colour {
            Colour::Fitted => {
                report.colours += 1;
                report.colour_fitted += 1;
            }
            Colour::Template => {
                report.colours += 1;
                report.colour_by_template += 1;
            }
            Colour::Unknown => report.colour_unknown += 1,
            Colour::None | Colour::NotChecked => {}
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
            Holding::Unknown => {
                report.unknown += 1;
                if entity_named(save, item.holder) {
                    report.ambiguities.push(format!(
                        "item {id} is held by object {}, which is not an object that holds items",
                        item.holder
                    ));
                }
            }
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
            units,
            kotor_rows,
            tsl_slots,
            unresolved_parts,
            unlisted_rows,
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
