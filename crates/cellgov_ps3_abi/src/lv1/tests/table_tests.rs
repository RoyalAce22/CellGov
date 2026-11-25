//! The hypercall table facts: the site rows, the live 4.93 map and its
//! lookup.

use super::{
    table_entry, table_site, TableEntry, HYPERCALL_TABLE_ENTRIES, HYPERCALL_TABLE_SITES,
    INVALID_HYPERCALL_R3, LIVE_TABLE_FIRMWARE, LIVE_TABLE_HANDLERS,
};
use crate::lv1::status::{zero_extended, LV1_NOT_IMPLEMENTED};

#[test]
fn the_invalid_hypercall_r3_is_not_implemented_zero_extended() {
    assert_eq!(INVALID_HYPERCALL_R3, zero_extended(LV1_NOT_IMPLEMENTED));
    assert_eq!(INVALID_HYPERCALL_R3, 0x0000_0000_FFFF_FFEC);
}

#[test]
fn every_site_puts_the_invalid_function_0x1f0_past_set_table() {
    assert_eq!(HYPERCALL_TABLE_SITES.len(), 5);
    for site in HYPERCALL_TABLE_SITES {
        assert_eq!(
            site.invalid_function,
            site.set_table + 0x1F0,
            "{}",
            site.firmware
        );
    }
}

#[test]
fn the_sites_ascend_by_firmware_and_each_is_found_by_its_version() {
    let versions: Vec<&str> = HYPERCALL_TABLE_SITES.iter().map(|s| s.firmware).collect();
    assert_eq!(versions, ["1.94", "3.15", "3.41", "3.70", "4.93"]);
    assert_eq!(table_site("4.93").map(|s| s.table_base), Some(0x372D08));
    assert_eq!(table_site("1.50"), None);
}

#[test]
fn the_live_table_names_151_handlers_ascending_below_the_entry_count() {
    assert_eq!(LIVE_TABLE_HANDLERS.len(), 151);
    assert!(LIVE_TABLE_HANDLERS.windows(2).all(|w| w[0].0 < w[1].0));
    let last = LIVE_TABLE_HANDLERS.last().map(|(n, _)| u64::from(*n));
    assert!(last < Some(HYPERCALL_TABLE_ENTRIES));
}

#[test]
fn no_live_entry_is_problem_state_callable() {
    let mut entries = 0;
    for number in 0..HYPERCALL_TABLE_ENTRIES {
        let entry = table_entry(LIVE_TABLE_FIRMWARE, number).expect("an entry below 256");
        assert!(!entry.problem_state_callable(), "entry {number}");
        entries += 1;
    }
    assert_eq!(entries, 256);
}

#[test]
fn a_flagged_handler_is_problem_state_callable() {
    assert!(TableEntry::Handler(0x2EBD39).problem_state_callable());
    assert!(!TableEntry::Handler(0x2EBD38).problem_state_callable());
    assert!(!TableEntry::Invalid.problem_state_callable());
}

#[test]
fn the_lookup_gives_a_handler_an_invalid_entry_or_nothing() {
    assert_eq!(
        table_entry("4.93", 246),
        Some(TableEntry::Handler(0x308A08))
    );
    assert_eq!(table_entry("4.93", 14), Some(TableEntry::Invalid));
    assert_eq!(
        table_entry("4.93", 255),
        Some(TableEntry::Handler(0x2EBE34))
    );
    assert_eq!(table_entry("4.93", 256), None);
    assert_eq!(table_entry("3.70", 14), None);
}

#[test]
fn the_live_table_holds_105_invalid_entries() {
    let invalid = (0..HYPERCALL_TABLE_ENTRIES)
        .filter(|n| table_entry(LIVE_TABLE_FIRMWARE, *n) == Some(TableEntry::Invalid))
        .count();
    assert_eq!(invalid, 105);
}
