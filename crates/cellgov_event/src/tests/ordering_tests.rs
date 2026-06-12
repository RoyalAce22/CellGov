//! OrderingKey four-tier comparison: timestamp, then priority, source, and sequence.

use super::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

fn key(t: u64, p: PriorityClass, u: u64, s: u64) -> OrderingKey {
    OrderingKey::new(
        GuestTicks::new(t),
        p,
        UnitId::new(u),
        SequenceNumber::new(s),
    )
}

fn hash_of(k: &OrderingKey) -> u64 {
    let mut h = DefaultHasher::new();
    k.hash(&mut h);
    h.finish()
}

#[test]
fn tier_one_timestamp_dominates_everything() {
    let earlier = key(10, PriorityClass::Background, 999, 999);
    let later = key(11, PriorityClass::Critical, 0, 0);
    assert!(earlier < later);
}

#[test]
fn tier_two_priority_breaks_timestamp_tie() {
    let crit = key(50, PriorityClass::Critical, 0, 0);
    let high = key(50, PriorityClass::High, 0, 0);
    let normal = key(50, PriorityClass::Normal, 0, 0);
    let bg = key(50, PriorityClass::Background, 999, 999);
    assert!(crit < high);
    assert!(high < normal);
    assert!(normal < bg);
}

#[test]
fn tier_three_source_breaks_priority_tie() {
    let lo = key(50, PriorityClass::Normal, 1, 999);
    let hi = key(50, PriorityClass::Normal, 2, 0);
    assert!(lo < hi);
}

#[test]
fn tier_four_sequence_breaks_source_tie() {
    let lo = key(50, PriorityClass::Normal, 7, 1);
    let hi = key(50, PriorityClass::Normal, 7, 2);
    assert!(lo < hi);
}

#[test]
fn equal_keys_compare_equal() {
    let a = key(50, PriorityClass::High, 3, 9);
    let b = key(50, PriorityClass::High, 3, 9);
    assert_eq!(a, b);
    assert_eq!(a.cmp(&b), std::cmp::Ordering::Equal);
}

/// `PartialEq`/`Hash` agreement; pins against a future manual
/// impl drifting from the derive.
#[test]
fn equal_keys_produce_equal_hashes() {
    let a = key(50, PriorityClass::High, 3, 9);
    let b = key(50, PriorityClass::High, 3, 9);
    assert_eq!(hash_of(&a), hash_of(&b));
}

/// Catches a future manual `Clone` that silently drops a field.
#[test]
fn copy_preserves_ordering_identity() {
    let a = key(50, PriorityClass::High, 3, 9);
    let b = a;
    assert_eq!(a.cmp(&b), std::cmp::Ordering::Equal);
    assert_eq!(hash_of(&a), hash_of(&b));
}

/// A future move to wrapping arithmetic on `GuestTicks` would
/// silently invert this and break replay determinism.
#[test]
fn max_timestamp_compares_above_zero() {
    let lo = key(0, PriorityClass::default(), 0, 0);
    let hi = key(u64::MAX, PriorityClass::default(), 0, 0);
    assert!(lo < hi);
}

#[test]
fn explicit_origin_key_compares_lowest() {
    let origin = OrderingKey::new(
        GuestTicks::ZERO,
        PriorityClass::default(),
        UnitId::new(0),
        SequenceNumber::ZERO,
    );
    let later = key(1, PriorityClass::default(), 0, 0);
    assert!(origin < later);
}

#[test]
fn ordering_is_total_across_a_mixed_set() {
    let mut keys = [
        key(2, PriorityClass::Normal, 0, 0),
        key(1, PriorityClass::Critical, 99, 99),
        key(1, PriorityClass::Background, 5, 0),
        key(1, PriorityClass::Background, 5, 1),
        key(1, PriorityClass::Background, 4, 7),
        key(1, PriorityClass::Normal, 0, 0),
    ];
    keys.sort();
    let expected = [
        key(1, PriorityClass::Critical, 99, 99),
        key(1, PriorityClass::Normal, 0, 0),
        key(1, PriorityClass::Background, 4, 7),
        key(1, PriorityClass::Background, 5, 0),
        key(1, PriorityClass::Background, 5, 1),
        key(2, PriorityClass::Normal, 0, 0),
    ];
    assert_eq!(keys, expected);
}
