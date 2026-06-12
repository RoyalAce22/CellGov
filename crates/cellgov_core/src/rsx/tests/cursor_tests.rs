//! RSX FIFO cursor put/get/reference semantics and per-field state-hash sensitivity.

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
    assert_eq!(pristine.state_hash(), set_to_zero.state_hash());
}

#[test]
fn empty_cursor_hash_is_stable() {
    let a = RsxFifoCursor::new();
    let b = RsxFifoCursor::new();
    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_deterministic_across_identical_cursors() {
    let mut a = RsxFifoCursor::new();
    a.set_put(0xABCD);
    a.set_get(0x100);
    a.set_reference(0xFEEDFACE);

    let mut b = RsxFifoCursor::new();
    b.set_put(0xABCD);
    b.set_get(0x100);
    b.set_reference(0xFEEDFACE);

    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_distinguishes_each_field() {
    let mut base = RsxFifoCursor::new();
    base.set_put(1);

    let mut put_different = RsxFifoCursor::new();
    put_different.set_put(2);
    assert_ne!(
        put_different.state_hash(),
        base.state_hash(),
        "put distinguishes"
    );

    let mut get_different = RsxFifoCursor::new();
    get_different.set_put(1);
    get_different.set_get(1);
    assert_ne!(
        get_different.state_hash(),
        base.state_hash(),
        "get distinguishes"
    );

    let mut ref_different = RsxFifoCursor::new();
    ref_different.set_put(1);
    ref_different.set_reference(1);
    assert_ne!(
        ref_different.state_hash(),
        base.state_hash(),
        "reference distinguishes"
    );
}

#[test]
fn state_hash_distinguishes_raw_put_from_masked_equivalent() {
    let mut raw = RsxFifoCursor::new();
    raw.set_put(0x7FFF_FFFF);
    let mut masked = RsxFifoCursor::new();
    masked.set_put(0x7FFF_FFFF & 0xFFFF);
    assert_ne!(raw.state_hash(), masked.state_hash());
}

#[test]
fn empty_cursor_hash_golden() {
    const EXPECTED: u64 = 0xeca4_bd25_1670_946c;
    let actual = RsxFifoCursor::new().state_hash();
    assert_eq!(
        actual, EXPECTED,
        "empty cursor hash drift: got 0x{:016x}, expected 0x{:016x}",
        actual, EXPECTED
    );
}

#[test]
fn populated_cursor_hash_golden() {
    const EXPECTED: u64 = 0x3fed_cabe_847c_2bac;
    let mut cur = RsxFifoCursor::new();
    cur.set_put(1);
    cur.set_get(2);
    cur.set_reference(3);
    let actual = cur.state_hash();
    assert_eq!(
        actual, EXPECTED,
        "populated cursor hash drift: got 0x{:016x}, expected 0x{:016x}",
        actual, EXPECTED
    );
}
