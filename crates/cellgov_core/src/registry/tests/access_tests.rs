//! Unit-registry lookup, iteration order, and runnable counting under status overrides.

use super::*;
use crate::registry::test_fixtures::{status_unit, CountingUnit};
use cellgov_exec::ExecutionContext;
use cellgov_mem::GuestMemory;
use cellgov_time::{Budget, InstructionCost};

#[test]
fn new_is_empty() {
    let r = UnitRegistry::new();
    assert!(r.is_empty());
    assert_eq!(r.len(), 0);
    assert_eq!(r.ids().count(), 0);
}

#[test]
fn get_returns_registered_unit() {
    let mut r = UnitRegistry::new();
    let id = r.register_with(|id| CountingUnit { id, steps: 0 });
    let u = r.get(id).expect("present");
    assert_eq!(u.unit_id(), id);
    assert_eq!(u.status(), UnitStatus::Runnable);
}

#[test]
fn get_missing_is_none() {
    let r = UnitRegistry::new();
    assert!(r.get(UnitId::new(99)).is_none());
}

#[test]
fn get_mut_drives_run_until_yield() {
    let mut r = UnitRegistry::new();
    let id = r.register_with(|id| CountingUnit { id, steps: 0 });
    let mem = GuestMemory::new(8);
    let ctx = ExecutionContext::new(&mem);
    let u = r.get_mut(id).expect("present");
    let mut effects = Vec::new();
    let step = u.run_until_yield(Budget::new(5), &ctx, &mut effects);
    assert_eq!(step.consumed_cost, InstructionCost::new(5));
    assert_eq!(effects.len(), 1);
}

#[test]
fn iter_is_in_id_order() {
    let mut r = UnitRegistry::new();
    for _ in 0..4 {
        r.register_with(|id| CountingUnit { id, steps: 0 });
    }
    let ids: Vec<u64> = r.iter().map(|(id, _)| id.raw()).collect();
    assert_eq!(ids, vec![0, 1, 2, 3]);
}

#[test]
fn ids_iterator_matches_registration_order() {
    let mut r = UnitRegistry::new();
    for _ in 0..3 {
        r.register_with(|id| CountingUnit { id, steps: 0 });
    }
    let collected: Vec<UnitId> = r.ids().collect();
    assert_eq!(
        collected,
        vec![UnitId::new(0), UnitId::new(1), UnitId::new(2)]
    );
}

#[test]
fn iter_mut_can_step_every_unit() {
    let mut r = UnitRegistry::new();
    for _ in 0..3 {
        r.register_with(|id| CountingUnit { id, steps: 0 });
    }
    let mem = GuestMemory::new(8);
    let ctx = ExecutionContext::new(&mem);
    let mut total = 0;
    let mut effects = Vec::new();
    for (_, u) in r.iter_mut() {
        effects.clear();
        u.run_until_yield(Budget::new(1), &ctx, &mut effects);
        total += effects.len();
    }
    assert_eq!(total, 3);
}

#[test]
fn runnable_ids_respects_override_transitions() {
    let mut r = UnitRegistry::new();
    let (h0, f0) = status_unit(UnitStatus::Runnable);
    let (h1, f1) = status_unit(UnitStatus::Blocked);
    let (h2, f2) = status_unit(UnitStatus::Runnable);
    r.register_with(f0);
    r.register_with(f1);
    r.register_with(f2);
    assert_eq!(r.runnable_ids().count(), 2);
    r.set_status_override(UnitId::new(0), UnitStatus::Blocked);
    assert_eq!(r.runnable_ids().count(), 1);
    h1.set(UnitStatus::Runnable);
    assert_eq!(r.runnable_ids().count(), 2);
    r.clear_status_override(UnitId::new(0));
    assert_eq!(r.runnable_ids().count(), 3);
    let _ = (h0, h2);
}
