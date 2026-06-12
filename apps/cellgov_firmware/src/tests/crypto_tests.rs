//! Application-key table lookup by SELF revision, including table gaps and out-of-range revisions.

use super::*;

#[test]
fn app_key_lookup_returns_expected_entries() {
    let k = app_key_for_revision(0x0000).expect("revision 0x0000 present");
    assert_eq!(k.erk[0], 0x95);
    assert_eq!(k.riv[0], 0x4A);
}

#[test]
fn app_key_lookup_handles_gaps() {
    assert!(app_key_for_revision(0x0012).is_none());
    assert!(app_key_for_revision(0x0015).is_none());
}

#[test]
fn app_key_lookup_returns_none_past_table() {
    assert!(app_key_for_revision(0x9999).is_none());
}

#[test]
fn revision_001c_key_matches_reference() {
    let k = app_key_for_revision(0x001C).expect("revision 0x001C present");
    assert_eq!(k.erk[0], 0xCF);
    assert_eq!(k.erk[1], 0xF0);
    assert_eq!(k.riv[0], 0xFD);
}
