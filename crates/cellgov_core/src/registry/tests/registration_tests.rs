//! Sequential id assignment and factory-failure behavior in unit registration.

use super::*;
use crate::registry::test_fixtures::{CountingUnit, LyingUnit};

#[test]
fn register_assigns_sequential_ids() {
    let mut r = UnitRegistry::new();
    let a = r.register_with(|id| CountingUnit { id, steps: 0 });
    let b = r.register_with(|id| CountingUnit { id, steps: 0 });
    let c = r.register_with(|id| CountingUnit { id, steps: 0 });
    assert_eq!(a, UnitId::new(0));
    assert_eq!(b, UnitId::new(1));
    assert_eq!(c, UnitId::new(2));
    assert_eq!(r.len(), 3);
}

#[test]
#[should_panic(expected = "registered unit reported")]
fn factory_id_mismatch_panics() {
    let mut r = UnitRegistry::new();
    r.register_with(|_assigned| LyingUnit);
}

/// AssertUnwindSafe holds only while `register_with` performs no
/// `&mut self` mutation before `factory(id)` returns. Adding any
/// such mutation before the factory call regresses this test's
/// soundness silently.
#[test]
fn factory_panic_does_not_burn_next_id() {
    let mut r = UnitRegistry::new();

    let id0 = r.register_with(|id| CountingUnit { id, steps: 0 });
    assert_eq!(id0, UnitId::new(0));

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        r.register_with::<CountingUnit, _>(|_id| panic!("synthetic factory failure"));
    }));
    assert!(result.is_err(), "factory must have panicked");

    let id1 = r.register_with(|id| CountingUnit { id, steps: 0 });
    assert_eq!(
        id1,
        UnitId::new(1),
        "next_id must not advance when a factory panics -- \
         a hole in the id sequence silently changes replay hashes"
    );
    assert_eq!(r.len(), 2);
}
