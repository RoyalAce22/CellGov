//! StateHash construction, ordering, and value-equality semantics.

use super::*;

#[test]
fn zero_is_origin() {
    assert_eq!(StateHash::ZERO, StateHash::new(0));
    assert_eq!(StateHash::default(), StateHash::ZERO);
}

#[test]
fn roundtrip() {
    assert_eq!(
        StateHash::new(0xdead_beef_cafe_babe).raw(),
        0xdead_beef_cafe_babe
    );
}

#[test]
fn equality_compares_value() {
    let a = StateHash::new(42);
    let b = StateHash::new(42);
    let c = StateHash::new(43);
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn ordering_is_total() {
    assert!(StateHash::new(1) < StateHash::new(2));
    assert!(StateHash::new(99) > StateHash::new(50));
}

#[test]
fn copy_semantics_hold() {
    let h = StateHash::new(7);
    let g = h;
    assert_eq!(h, g);
}
