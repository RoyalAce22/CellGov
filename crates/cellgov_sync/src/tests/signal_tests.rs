//! SignalRegister OR-accumulation semantics -- idempotence, commutativity, and clear.

use super::*;

#[test]
fn roundtrip() {
    assert_eq!(SignalId::new(99).raw(), 99);
}

#[test]
fn display_emits_raw_integer() {
    assert_eq!(format!("{}", SignalId::new(42)), "42");
}

#[test]
fn new_register_is_zero() {
    let r = SignalRegister::new();
    assert_eq!(r.value(), 0);
    assert_eq!(SignalRegister::default(), r);
}

#[test]
fn with_value_constructs_pre_loaded_register() {
    let r = SignalRegister::with_value(0xdead_beef);
    assert_eq!(r.value(), 0xdead_beef);
}

#[test]
fn or_in_sets_bits() {
    let mut r = SignalRegister::new();
    assert_eq!(r.or_in(0b0001), 0b0001);
    assert_eq!(r.or_in(0b0010), 0b0011);
    assert_eq!(r.or_in(0b1000), 0b1011);
    assert_eq!(r.value(), 0b1011);
}

#[test]
fn or_in_is_idempotent_under_repeated_identical_updates() {
    let mut r = SignalRegister::new();
    r.or_in(0xff);
    let after_first = r.value();
    r.or_in(0xff);
    r.or_in(0xff);
    assert_eq!(r.value(), after_first);
}

#[test]
fn or_in_is_monotonic_in_bits_set() {
    let mut r = SignalRegister::new();
    let mut bits_before = 0u32;
    for v in [0x01, 0x10, 0x100, 0x1000, 0x10000] {
        r.or_in(v);
        let bits_after = r.value().count_ones();
        assert!(bits_after >= bits_before);
        bits_before = bits_after;
    }
}

#[test]
fn or_in_is_commutative() {
    let mut a = SignalRegister::new();
    a.or_in(0x0f);
    a.or_in(0xf0);
    let mut b = SignalRegister::new();
    b.or_in(0xf0);
    b.or_in(0x0f);
    assert_eq!(a.value(), b.value());
    assert_eq!(a.value(), 0xff);
}

#[test]
fn clear_resets_to_zero() {
    let mut r = SignalRegister::with_value(0xffff_ffff);
    r.clear();
    assert_eq!(r.value(), 0);
}

#[test]
fn or_in_zero_is_a_noop() {
    let mut r = SignalRegister::with_value(0xa5a5);
    let after = r.or_in(0);
    assert_eq!(after, 0xa5a5);
    assert_eq!(r.value(), 0xa5a5);
}
