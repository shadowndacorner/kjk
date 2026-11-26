//! The recovery accounting check (REC-01): a save's holdings counted once,
//! without writing or changing it.

use crate::recovery::inspect;
use sim::inventory::{COLOUR_SLOT, Catalog, Item, ItemFacts, PARTY, slots};

/// The accounting of a save with a party-loose stack, an equipped saber, a
/// carried medpac, an unknown holder and a K1 upgrade row.
pub fn accounting() -> Result<String, String> {
    let (_, mut save) = sim::fixtures::kitchen_sink();
    let player = crate::recovery::first_seat_holder(&save)
        .ok_or("no first seat")?
        .0;
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
    ]);
    let item = |template: &str,
                holder: u32,
                slot: Option<u8>,
                stack: u16,
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
            infinite: false,
            non_equippable: false,
            upgrades,
            fitted,
        }
    };
    save.script
        .items
        .insert(1, item("medpac", PARTY, None, 2, 0, Vec::new()));
    save.script.items.insert(
        2,
        item(
            "saber",
            player,
            Some(slots::RIGHT_WEAPON),
            1,
            0,
            vec!["crystal".into()],
        ),
    );
    save.script
        .items
        .insert(3, item("carried", player, None, 1, 0, Vec::new()));
    save.script.items.insert(
        4,
        item("not_in_catalog", 0x7d00_9999, None, 1, 0, Vec::new()),
    );
    save.script
        .items
        .insert(5, item("k1", PARTY, None, 1, 2, Vec::new()));
    let before = save.clone();
    let report = inspect(&save, Some(&catalog));
    let unchanged = save == before;
    // Reading one item by its number is part of the same read-only promise.
    let read = crate::recovery::item(&save, 2).is_some_and(|held| held.template == "saber");
    let counted = read
        && report.items.len() == 5
        && report.units == 6
        && report.party_loose == 2
        && report.carried == 1
        && report.equipped == 1
        && report.other == 1
        && report.k1_rows == 1
        && report.k2_slots == 1
        && report.colours == 1
        && report.catalog_unknown == 1
        && report.unknown_holders == vec![0x7d00_9999]
        && report.ambiguities.is_empty();
    if !unchanged || !counted {
        return Err(format!(
            "read {}, unchanged {}, items {}, units {}, party {}, carried {}, equipped {}, other {}, rows {}, slots {}, colours {}, unknown {}, holders {:?}, ambiguities {:?}",
            read,
            unchanged,
            report.items.len(),
            report.units,
            report.party_loose,
            report.carried,
            report.equipped,
            report.other,
            report.k1_rows,
            report.k2_slots,
            report.colours,
            report.catalog_unknown,
            report.unknown_holders,
            report.ambiguities
        ));
    }
    Ok("one party stack, one equipped saber, one carried medpac, one unknown holder, one K1 row and one fitted colour counted once; the save unchanged, and one item read by its number".into())
}
