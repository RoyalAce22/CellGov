//! UnitId, EventId, and SequenceNumber round-trips, total ordering, and hash/eq agreement.

use super::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

fn hash<T: Hash>(t: &T) -> u64 {
    let mut h = DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

#[test]
fn unit_id_roundtrip() {
    assert_eq!(UnitId::new(7).raw(), 7);
}

#[test]
fn unit_id_ordering_is_total() {
    assert!(UnitId::new(1) < UnitId::new(2));
    assert_eq!(UnitId::new(5), UnitId::new(5));
}

#[test]
fn unit_id_hash_matches_eq() {
    assert_eq!(hash(&UnitId::new(7)), hash(&UnitId::new(7)));
    assert_ne!(hash(&UnitId::new(7)), hash(&UnitId::new(8)));
}

#[test]
fn unit_id_copy_preserves_value() {
    let a = UnitId::new(42);
    let b = a;
    assert_eq!(a, b);
    assert_eq!(a.raw(), 42);
    assert_eq!(hash(&a), hash(&b));
}

#[test]
fn event_id_roundtrip() {
    assert_eq!(EventId::new(42).raw(), 42);
}

#[test]
fn event_id_ordering_is_total() {
    assert!(EventId::new(10) < EventId::new(11));
}

#[test]
fn event_id_hash_matches_eq() {
    assert_eq!(hash(&EventId::new(42)), hash(&EventId::new(42)));
    assert_ne!(hash(&EventId::new(42)), hash(&EventId::new(43)));
}

/// Derived `Hash` on a single-field newtype delegates to the
/// inner `u64`, so wrappers with the same raw value DO hash
/// identically. The wall against `UnitId`/`EventId` confusion
/// lives at the type level, not the hash level.
#[test]
fn unit_and_event_ids_share_hash_when_raw_collides() {
    assert_eq!(hash(&UnitId::new(7)), hash(&EventId::new(7)));
    assert_eq!(hash(&UnitId::new(7).raw()), hash(&EventId::new(7).raw()));
}

#[test]
fn sequence_zero_is_origin() {
    assert_eq!(SequenceNumber::ZERO, SequenceNumber::new(0));
    assert_eq!(SequenceNumber::default(), SequenceNumber::ZERO);
}

#[test]
fn sequence_next_advances_by_one() {
    assert_eq!(SequenceNumber::ZERO.next(), Some(SequenceNumber::new(1)));
    assert_eq!(
        SequenceNumber::new(99).next(),
        Some(SequenceNumber::new(100))
    );
}

#[test]
fn sequence_chain_from_zero_is_monotonic() {
    let s0 = SequenceNumber::ZERO;
    let s1 = s0.next().unwrap();
    let s2 = s1.next().unwrap();
    let s3 = s2.next().unwrap();
    assert_eq!(s3, SequenceNumber::new(3));
    assert!(s0 < s1 && s1 < s2 && s2 < s3);
}

#[test]
fn sequence_next_at_max_is_none() {
    assert_eq!(SequenceNumber::new(u64::MAX).next(), None);
}

#[test]
fn sequence_ordering_is_total_and_monotonic() {
    assert!(SequenceNumber::new(1) < SequenceNumber::new(2));
}

#[test]
fn sequence_hash_matches_eq() {
    assert_eq!(hash(&SequenceNumber::new(5)), hash(&SequenceNumber::new(5)));
    assert_ne!(hash(&SequenceNumber::new(5)), hash(&SequenceNumber::new(6)));
}
