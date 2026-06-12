//! Decision-log capture from a live runtime: branching appears only with multiple runnable units.

use super::*;
use cellgov_core::Runtime;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

#[test]
fn two_units_produces_branching_point() {
    let mem = GuestMemory::new(64);
    let mut rt = Runtime::new(mem, Budget::new(100), 100);

    rt.registry_mut()
        .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
    rt.registry_mut()
        .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));

    let log = observe_decisions(&mut rt);
    assert!(
        log.len() >= 2,
        "expected at least 2 steps, got {}",
        log.len()
    );
    assert!(
        log.points()[0].is_branching(),
        "first step should be a branching point (2 runnable)"
    );
    assert!(
        log.branching_count() >= 1,
        "expected at least 1 branching point, got {}",
        log.branching_count()
    );
}

#[test]
fn single_unit_no_branching() {
    let mem = GuestMemory::new(64);
    let mut rt = Runtime::new(mem, Budget::new(100), 100);

    rt.registry_mut()
        .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));

    let log = observe_decisions(&mut rt);
    assert_eq!(log.len(), 1);
    assert_eq!(log.branching_count(), 0);
}
