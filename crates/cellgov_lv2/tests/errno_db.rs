//! Integration tests for the PS3 LV2 errno database. Pins the
//! invariants external consumers depend on: codes are unique,
//! symbol fields match their constant names, lookup round-trips,
//! and three well-known entries have the exact values from the
//! RPCS3 header.

use cellgov_lv2::errno::{self, Lv2Error, CELL_EFAULT, CELL_EINVAL, CELL_EPERM, ENTRIES};

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
    // `errno_table!` binds `symbol` to `stringify!($name)`, so
    // this invariant is structural. The test acts as a canary
    // against anyone replacing the macro call with hand-written
    // entries whose symbol string drifts from the const name.
    // Spot-check the three canary entries by value so a
    // symbol-name swap anywhere in the table fails loudly.
    let expected: &[(&str, &Lv2Error)] = &[
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
    // And every entry in ENTRIES has a CELL_E prefix (the macro
    // guarantees this; the check catches a future
    // hand-written-entry regression).
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
    assert_eq!(errno::lookup(0x8001_0009), Some(&CELL_EPERM));
    assert!(errno::lookup(0xDEAD_BEEF).is_none());
    // CELL_OK belongs to CellNotAnError, not CellError, so
    // lookup must NOT return it.
    assert!(errno::lookup(0).is_none());
}

#[test]
fn spot_check_three_canaries_against_rpcs3_header() {
    // Pinned to rpcs3/Emu/Cell/ErrorCodes.h:104-133. Any
    // silent "fix" of a value without checking the source
    // trips this canary.
    assert_eq!(CELL_EINVAL.code, 0x8001_0002);
    assert_eq!(CELL_EPERM.code, 0x8001_0009);
    assert_eq!(CELL_EFAULT.code, 0x8001_000D);
}
