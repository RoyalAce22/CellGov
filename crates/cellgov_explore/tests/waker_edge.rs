//! Only a wake that ended a park is ordered before the step it
//! released.
//!
//! No footprint pair orders those two: the wake carries a target and
//! the released step emits no wait. Happens-before joins the waker's
//! clock into the released step's clock instead.
//!
//! The two directions cost different things:
//!
//! - A missing edge lets a race ask for a reversal whose sequence names
//!   the woken unit where it is still parked. The search drops that
//!   branch and withdraws its class count, so the same executions run
//!   and the claim over them goes.
//! - An edge from a wake that released nothing orders a pair the
//!   schedule leaves free, which removes a race and the reversal it
//!   owed.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_explore::execution::Execution;
use cellgov_explore::observer::observe_decisions;
use cellgov_explore::util::StopReason;
use cellgov_explore::{explore_optimal, ExplorationConfig};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

const PARKED: UnitId = UnitId::new(0);
const WAKER: UnitId = UnitId::new(1);
/// Unit 0 of the second workload below, which parks on nothing.
const RUNNING: UnitId = UnitId::new(0);

/// One word, three writers, and a park in the middle of them: the races
/// between unit 1 and unit 2 ask for reversals whose sequences run
/// through the write unit 0 makes once the wake releases it.
fn wake_between_writers() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(1), 400);
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::Barrier { barrier: 0 },
                FakeOp::LoadImm(0xA0),
                FakeOp::SharedStore { addr: 0, len: 4 },
                FakeOp::End,
            ],
        )
    });
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::LoadImm(0xB0),
                FakeOp::SharedStore { addr: 0, len: 4 },
                FakeOp::Wake { unit: PARKED.raw() },
                FakeOp::LoadImm(0xB1),
                FakeOp::SharedStore { addr: 0, len: 4 },
                FakeOp::End,
            ],
        )
    });
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::LoadImm(0xC0),
                FakeOp::SharedStore { addr: 0, len: 4 },
                FakeOp::End,
            ],
        )
    });
    rt
}

#[test]
fn the_wake_precedes_the_step_it_enabled() {
    let mut rt = wake_between_writers();
    let (log, stop) = observe_decisions(&mut rt);
    assert_eq!(stop, StopReason::Stalled);
    let execution = Execution::from_log(&log);
    let relation = execution.happens_before();

    let wake = execution
        .events()
        .iter()
        .find(|event| event.footprint.wake_targets.contains(&PARKED))
        .expect("unit 1 wakes unit 0");
    let released = execution
        .events()
        .iter()
        .find(|event| event.id.unit == PARKED && event.id.index > wake.id.index)
        .expect("unit 0 runs again once released");

    assert_eq!(wake.id.unit, WAKER);
    assert!(
        relation.precedes(wake.id, released.id),
        "no footprint pairs these two, so the join is the only thing that orders them",
    );
}

/// Eleven classes: one order over the four writes, times where the park
/// falls against the wake.
///
/// Park before wake: unit 1's first write leads its own second and unit
/// 0's, those two fall either way, and unit 2's write takes any of four
/// slots -- 2 x 4 = 8. Wake before park: the wake releases nothing,
/// unit 0 parks for good and never writes, and unit 2's write takes any
/// of three slots -- 3.
#[test]
fn the_search_drops_no_branch_over_a_workload_with_a_park() {
    let config = ExplorationConfig {
        max_schedules: 5_000,
        max_steps_per_run: 500,
    };
    let result = explore_optimal(wake_between_writers, &config);
    assert_eq!(result.baseline_stop, StopReason::Stalled);
    assert!(
        !result.bounds_hit,
        "no bound stopped this search, so a withdrawn count would be a drop",
    );
    assert_eq!(
        (result.schedules.len() + 1, result.classes_explored),
        (11, Some(11)),
        "one execution per class, and the count stands for every one",
    );
}

/// A wake that names a unit no park holds; unit 1 writes nothing, so the
/// wake is the only thing that could order the two units.
fn wake_a_running_unit() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(1), 400);
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::LoadImm(0xA0),
                FakeOp::SharedStore { addr: 0, len: 4 },
                FakeOp::End,
            ],
        )
    });
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::Wake {
                    unit: RUNNING.raw(),
                },
                FakeOp::End,
            ],
        )
    });
    rt
}

#[test]
fn a_wake_that_released_no_park_precedes_no_step() {
    let mut rt = wake_a_running_unit();
    let (log, stop) = observe_decisions(&mut rt);
    assert_eq!(stop, StopReason::Stalled);
    let execution = Execution::from_log(&log);
    let relation = execution.happens_before();

    assert!(
        execution
            .events()
            .iter()
            .all(|event| event.footprint.wait_units.is_empty()),
        "no step parks anything, so no wake in this run ended a park",
    );

    let wake = execution
        .events()
        .iter()
        .find(|event| event.footprint.wake_targets.contains(&RUNNING))
        .expect("unit 1 wakes unit 0");
    let next = execution
        .events()
        .iter()
        .find(|event| event.id.unit == RUNNING && event.id.index > wake.id.index)
        .expect("unit 0 still has a step left when the wake lands");

    assert!(
        !relation.precedes(wake.id, next.id),
        "the wake found its target runnable, so the schedule forces no order here",
    );
}
