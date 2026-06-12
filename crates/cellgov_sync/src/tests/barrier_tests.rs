//! BarrierId newtype semantics -- raw round trip, hashing, copy, and display.

use super::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

fn hash<T: Hash>(t: &T) -> u64 {
    let mut h = DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

#[test]
fn roundtrip() {
    assert_eq!(BarrierId::new(13).raw(), 13);
}

#[test]
fn hash_matches_eq() {
    assert_eq!(hash(&BarrierId::new(7)), hash(&BarrierId::new(7)));
    assert_ne!(hash(&BarrierId::new(7)), hash(&BarrierId::new(8)));
}

#[test]
fn copy_preserves_value() {
    let a = BarrierId::new(5);
    let b = a;
    assert_eq!(a, b);
    assert_eq!(a.raw(), 5);
}

#[test]
fn max_id_roundtrips() {
    assert_eq!(BarrierId::new(u64::MAX).raw(), u64::MAX);
}

#[test]
fn display_emits_raw_integer() {
    assert_eq!(format!("{}", BarrierId::new(42)), "42");
}
