//! PS3 LV2 errno database tests.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: .unwrap() panics on unexpected failure are the right behavior"
)]

use cellgov_ps3_abi::cell_errors::{
    self, Lv2ErrCode, CELL_EFAULT, CELL_EINVAL, CELL_EPERM, ENTRIES,
};

#[test]
fn every_code_is_unique() {
    let mut seen = std::collections::BTreeSet::new();
    for entry in ENTRIES {
        assert!(
            seen.insert(entry.code),
            "duplicate code 0x{:08x} ({})",
            entry.code,
            entry.symbol,
        );
    }
}

#[test]
fn every_symbol_matches_its_constant_name() {
    let expected: &[(&str, &Lv2ErrCode)] = &[
        ("CELL_EINVAL", &CELL_EINVAL),
        ("CELL_EPERM", &CELL_EPERM),
        ("CELL_EFAULT", &CELL_EFAULT),
    ];
    for (name, entry) in expected {
        assert_eq!(
            entry.symbol, *name,
            "symbol field of {} does not match its const name",
            name,
        );
    }
    for entry in ENTRIES {
        assert!(
            entry.symbol.starts_with("CELL_E"),
            "entry symbol {:?} lacks CELL_E prefix",
            entry.symbol,
        );
    }
}

#[test]
fn lookup_hits_known_code_and_misses_unknown() {
    assert_eq!(cell_errors::lookup(0x8001_0009), Some(&CELL_EPERM));
    assert!(cell_errors::lookup(0xDEAD_BEEF).is_none());
    // CELL_OK belongs to CellNotAnError, not CellError.
    assert!(cell_errors::lookup(0).is_none());
}

#[test]
fn spot_check_three_canaries_against_rpcs3_header() {
    assert_eq!(CELL_EINVAL.code, 0x8001_0002);
    assert_eq!(CELL_EPERM.code, 0x8001_0009);
    assert_eq!(CELL_EFAULT.code, 0x8001_000D);
}
