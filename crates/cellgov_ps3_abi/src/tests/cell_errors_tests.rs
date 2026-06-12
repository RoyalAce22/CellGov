//! CELL errno table integrity -- code uniqueness, symbol prefix, and lookup behavior.

use super::*;
use std::collections::BTreeSet;

#[test]
fn every_code_is_unique() {
    let mut seen = BTreeSet::new();
    for entry in ENTRIES {
        assert!(
            seen.insert(entry.code),
            "duplicate errno code 0x{:08x} ({})",
            entry.code,
            entry.symbol,
        );
    }
}

#[test]
fn every_symbol_has_cell_e_prefix() {
    for entry in ENTRIES {
        assert!(
            entry.symbol.starts_with("CELL_E"),
            "symbol {:?} (code 0x{:08x}) does not start \
             with CELL_E",
            entry.symbol,
            entry.code,
        );
    }
}

#[test]
fn lookup_hits_and_misses() {
    assert_eq!(lookup(0x8001_0009), Some(&CELL_EPERM));
    assert!(lookup(0xDEAD_BEEF).is_none());
    assert!(lookup(0).is_none());
}
