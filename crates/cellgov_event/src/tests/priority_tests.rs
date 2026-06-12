//! PriorityClass locked discriminants, declaration-order Ord, and the Normal default.

use super::*;

#[test]
fn variant_order_is_locked() {
    assert!(PriorityClass::Critical < PriorityClass::High);
    assert!(PriorityClass::High < PriorityClass::Normal);
    assert!(PriorityClass::Normal < PriorityClass::Background);
}

#[test]
fn discriminants_are_locked() {
    assert_eq!(PriorityClass::Critical as u8, 0);
    assert_eq!(PriorityClass::High as u8, 1);
    assert_eq!(PriorityClass::Normal as u8, 2);
    assert_eq!(PriorityClass::Background as u8, 3);
}

/// First-variant fallback would be `Critical`, the worst
/// possible default. Pins the `#[default]` annotation in place.
#[test]
fn default_is_normal_not_first_variant() {
    assert_eq!(PriorityClass::default(), PriorityClass::Normal);
    assert_ne!(PriorityClass::default(), PriorityClass::Critical);
}

#[test]
fn equality_is_reflexive() {
    assert_eq!(PriorityClass::High, PriorityClass::High);
    assert_ne!(PriorityClass::High, PriorityClass::Critical);
}

/// Derived `Ord` follows declaration order; `repr(u8)` fixes
/// layout. A non-sequential discriminant (e.g. `Urgent = 5`
/// slotted between existing variants) would desync the two and
/// corrupt any trace wire format keyed on the `u8`. Iterates
/// `Self::VARIANTS` so a new variant is automatically covered.
#[test]
fn ord_agrees_with_discriminant_order() {
    use strum::VariantArray;
    for pair in PriorityClass::VARIANTS.windows(2) {
        assert!(
            pair[0] < pair[1],
            "{:?} (disc {}) should compare less than {:?} (disc {})",
            pair[0],
            pair[0] as u8,
            pair[1],
            pair[1] as u8
        );
        assert!(
            (pair[0] as u8) < (pair[1] as u8),
            "discriminant order must match Ord order"
        );
    }
}

/// Localizes a future `Hash`/`PartialEq` drift to this type
/// rather than relying on transitive coverage from `OrderingKey`.
#[test]
fn equal_variants_produce_equal_hashes() {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    fn h(p: PriorityClass) -> u64 {
        let mut hasher = DefaultHasher::new();
        p.hash(&mut hasher);
        hasher.finish()
    }
    assert_eq!(h(PriorityClass::Normal), h(PriorityClass::Normal));
    assert_ne!(h(PriorityClass::Critical), h(PriorityClass::Background));
}

/// Trip-wire: the exhaustive `match` (no `_` arm) refuses to
/// compile when a fifth variant is added, forcing the
/// determinism-contract tests to be updated rather than
/// absorbed silently.
#[test]
fn variant_count_is_locked() {
    let disc = |p: PriorityClass| match p {
        PriorityClass::Critical => 0u8,
        PriorityClass::High => 1,
        PriorityClass::Normal => 2,
        PriorityClass::Background => 3,
    };
    assert_eq!(disc(PriorityClass::Critical), 0);
    assert_eq!(disc(PriorityClass::Background), 3);
}
