//! RSX FIFO cursor put/get/reference semantics, and the sensitivity of the sync-state term to each field.

use super::*;

#[test]
fn new_cursor_is_empty() {
    let cur = RsxFifoCursor::new();
    assert_eq!(cur.put(), 0);
    assert_eq!(cur.get(), 0);
    assert_eq!(cur.current_reference(), 0);
}

#[test]
fn set_put_stores_value_verbatim() {
    let mut cur = RsxFifoCursor::new();
    cur.set_put(0x1000);
    assert_eq!(cur.put(), 0x1000);
    cur.set_put(0xDEAD_BEEF);
    assert_eq!(cur.put(), 0xDEAD_BEEF);
}

#[test]
fn set_get_stores_value_verbatim() {
    let mut cur = RsxFifoCursor::new();
    cur.set_put(0x1000);
    cur.set_get(0x400);
    assert_eq!(cur.get(), 0x400);
    cur.set_get(0x1000);
    assert_eq!(cur.get(), 0x1000);
}

#[test]
fn set_get_accepts_value_past_put_without_assertion() {
    let mut cur = RsxFifoCursor::new();
    cur.set_put(0x100);
    cur.set_get(0x1_0000);
    assert_eq!(cur.get(), 0x1_0000);
    assert_eq!(cur.put(), 0x100);
}

#[test]
fn backward_set_put_does_not_auto_reset_get() {
    let mut cur = RsxFifoCursor::new();
    cur.set_put(0x2000);
    cur.set_get(0x1000);
    cur.set_put(0);
    assert_eq!(cur.put(), 0);
    assert_eq!(cur.get(), 0x1000, "get survives backward set_put");
}

#[test]
fn set_reference_updates_independent_field() {
    let mut cur = RsxFifoCursor::new();
    cur.set_put(0x2000);
    cur.set_get(0x1000);
    cur.set_reference(0xDEAD_BEEF);
    assert_eq!(cur.current_reference(), 0xDEAD_BEEF);
    assert_eq!(cur.put(), 0x2000);
    assert_eq!(cur.get(), 0x1000);
}

#[test]
fn reference_zero_is_indistinguishable_from_pristine() {
    let pristine = RsxFifoCursor::new();
    let mut set_to_zero = RsxFifoCursor::new();
    set_to_zero.set_reference(0);
    assert_eq!(pristine, set_to_zero);
    assert_eq!(pristine.sync_term(), set_to_zero.sync_term());
}

#[test]
fn empty_cursor_hash_is_stable() {
    let a = RsxFifoCursor::new();
    let b = RsxFifoCursor::new();
    assert_eq!(a.sync_term(), b.sync_term());
}

#[test]
fn sync_term_deterministic_across_identical_cursors() {
    let mut a = RsxFifoCursor::new();
    a.set_put(0xABCD);
    a.set_get(0x100);
    a.set_reference(0xFEEDFACE);

    let mut b = RsxFifoCursor::new();
    b.set_put(0xABCD);
    b.set_get(0x100);
    b.set_reference(0xFEEDFACE);

    assert_eq!(a.sync_term(), b.sync_term());
}

#[test]
fn sync_term_distinguishes_each_field() {
    let mut base = RsxFifoCursor::new();
    base.set_put(1);

    let mut put_different = RsxFifoCursor::new();
    put_different.set_put(2);
    assert_ne!(
        put_different.sync_term(),
        base.sync_term(),
        "put distinguishes"
    );

    let mut get_different = RsxFifoCursor::new();
    get_different.set_put(1);
    get_different.set_get(1);
    assert_ne!(
        get_different.sync_term(),
        base.sync_term(),
        "get distinguishes"
    );

    let mut ref_different = RsxFifoCursor::new();
    ref_different.set_put(1);
    ref_different.set_reference(1);
    assert_ne!(
        ref_different.sync_term(),
        base.sync_term(),
        "reference distinguishes"
    );
}

#[test]
fn sync_term_distinguishes_raw_put_from_masked_equivalent() {
    let mut raw = RsxFifoCursor::new();
    raw.set_put(0x7FFF_FFFF);
    let mut masked = RsxFifoCursor::new();
    masked.set_put(0x7FFF_FFFF & 0xFFFF);
    assert_ne!(raw.sync_term(), masked.sync_term());
}

/// Computed outside the crate from the SplitMix64 key stream of the
/// sync-state lanes.
#[test]
fn cursor_term_wire_format_golden() {
    assert_eq!(
        RsxFifoCursor::new().sync_term(),
        0x5ca5_cf3e_1811_cddb_b6b9_c23d_789c_9c53
    );
    let mut cur = RsxFifoCursor::new();
    cur.set_put(1);
    cur.set_get(2);
    cur.set_reference(3);
    assert_eq!(cur.sync_term(), 0x320a_0c24_8040_003d_9c1e_5c0f_c97f_82fd);
}
