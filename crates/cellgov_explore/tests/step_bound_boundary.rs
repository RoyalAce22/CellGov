//! The step cap at the exact length of a maximal execution.
//!
//! A cap refuses to start a step. An execution that retires exactly
//! `max_steps_per_run` steps and then has nothing left to run finished;
//! the cap did not stop it. Reporting a bound there withdraws a
//! complete answer, because `StopReason::is_truncated` holds for
//! `StepBound` and the run's committed hash leaves the outcome set.
//!
//! Three step loops read a cap -- the observer, the optimal search's
//! own, and `run_to_stall` -- and all three are held here. So is the
//! predicate they ask, against the `Runtime::step` that follows it.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_core::Runtime;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_explore::config::ExplorationConfig;
use cellgov_explore::explorer::explore_window;
use cellgov_explore::observer::{observe_decisions, observe_decisions_bounded};
use cellgov_explore::util::{run_to_stall, StopReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_testkit::world::{CountingUnit, DmaSubmitter, SleepingWriter, WritingUnit};
use cellgov_time::Budget;

/// Two units of three steps, so every maximal execution is six steps
/// and no schedule is shorter.
const MAXIMAL_STEPS: usize = 6;

/// A park and the step the warp lets the unit take once what it waits on
/// fires. Two, for either queue that holds a wake.
const WARP_STEPS: usize = 2;

fn workload() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(4), 200);
    rt.register_unit_with(|id| CountingUnit::new(id, 3));
    rt.register_unit_with(|id| CountingUnit::new(id, 3));
    rt
}

fn destination() -> ByteRange {
    ByteRange::new(GuestAddr::new(0), 4).unwrap()
}

/// `workload`'s length with a conflict in it, so the search has
/// alternates to replay against the same cap.
fn conflicting_workload() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(4), 200);
    rt.register_unit_with(|id| WritingUnit::of_value(id, 3, destination(), 0xaa));
    rt.register_unit_with(|id| WritingUnit::of_value(id, 3, destination(), 0xbb));
    rt
}

/// One unit that parks on a sleep and needs the time warp to finish:
/// between its two steps no unit is runnable and one is parked with a
/// queued deadline, the state a cap reads as work left.
fn warping_workload() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(4), 200);
    rt.register_unit_with(|id| SleepingWriter::new(id, 1, destination(), 0xbb));
    rt
}

/// One unit that parks on a transfer it put, so the wake source in the
/// queue is a DMA completion rather than a deadline.
fn dma_workload() -> Runtime {
    let source = ByteRange::new(GuestAddr::new(0), 4).unwrap();
    let landing = ByteRange::new(GuestAddr::new(128), 4).unwrap();
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(4), 200);
    rt.register_unit_with(|id| {
        DmaSubmitter::new(id, source, landing, vec![0xde, 0xad, 0xbe, 0xef])
    });
    rt
}

/// One unit that waits for a message nobody sends, so its park has no
/// wake source and the second step deadlocks.
fn parked_for_good() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(4), 200);
    let mailbox = rt.mailbox_registry_mut().register(4);
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::MailboxRecv {
                    mailbox: mailbox.raw(),
                },
                FakeOp::End,
            ],
        )
    });
    rt
}

#[test]
fn the_workload_is_exactly_as_long_as_the_cap_below() {
    let mut rt = workload();
    let (log, stop) = observe_decisions(&mut rt);
    assert_eq!(stop, StopReason::Stalled);
    assert_eq!(log.len(), MAXIMAL_STEPS);
}

#[test]
fn the_warping_workload_is_exactly_as_long_as_the_cap_below() {
    let mut rt = warping_workload();
    let (log, stop) = observe_decisions(&mut rt);
    assert_eq!(stop, StopReason::Stalled);
    assert_eq!(log.len(), WARP_STEPS);
}

#[test]
fn the_observer_at_the_exact_length_reports_the_stall_it_reached() {
    let mut rt = workload();
    let (log, stop) = observe_decisions_bounded(&mut rt, MAXIMAL_STEPS);
    assert_eq!(log.len(), MAXIMAL_STEPS);
    assert_eq!(
        stop,
        StopReason::Stalled,
        "nothing was left to run, so the cap started no step and stopped nothing",
    );
    assert!(
        !stop.is_truncated(),
        "a maximal execution's hash answers for the whole workload",
    );
}

#[test]
fn the_observer_one_step_short_reports_the_bound() {
    let mut rt = workload();
    let (log, stop) = observe_decisions_bounded(&mut rt, MAXIMAL_STEPS - 1);
    assert_eq!(log.len(), MAXIMAL_STEPS - 1);
    assert_eq!(
        stop,
        StopReason::StepBound,
        "a step was there to start and the cap refused it",
    );
    assert!(stop.is_truncated());
}

/// The cap lands where no unit is runnable and a wake is still coming; a
/// predicate that read the runnable set alone would let this run past
/// the cap.
#[test]
fn the_observer_at_a_wakeable_park_reports_the_bound() {
    let mut rt = warping_workload();
    let (log, stop) = observe_decisions_bounded(&mut rt, WARP_STEPS - 1);
    assert_eq!(log.len(), WARP_STEPS - 1);
    assert_eq!(stop, StopReason::StepBound);
    assert!(stop.is_truncated());
}

#[test]
fn the_observer_at_the_warping_length_reports_the_stall_it_reached() {
    let mut rt = warping_workload();
    let (log, stop) = observe_decisions_bounded(&mut rt, WARP_STEPS);
    assert_eq!(log.len(), WARP_STEPS);
    assert_eq!(stop, StopReason::Stalled);
}

/// Both stops leave no unit runnable; the queues separate them, and an
/// empty queue makes the execution maximal.
#[test]
fn the_observer_at_an_unwakeable_park_reports_the_deadlock() {
    let mut rt = parked_for_good();
    let (log, stop) = observe_decisions_bounded(&mut rt, 1);
    assert_eq!(log.len(), 1);
    assert_eq!(stop, StopReason::Deadlocked);
    assert!(!stop.is_truncated());
}

#[test]
fn run_to_stall_at_the_exact_length_reports_the_stall_it_reached() {
    let mut rt = workload();
    assert_eq!(run_to_stall(&mut rt, MAXIMAL_STEPS), StopReason::Stalled);
    let mut warping = warping_workload();
    assert_eq!(run_to_stall(&mut warping, WARP_STEPS), StopReason::Stalled);
    let mut short = warping_workload();
    assert_eq!(
        run_to_stall(&mut short, WARP_STEPS - 1),
        StopReason::StepBound,
    );
}

/// Nothing else ties the two together: the predicate reads the registry
/// and the two wake queues, while `step` asks the scheduler and warps.
#[test]
fn the_predicate_answers_for_the_step_that_follows_it() {
    let cases: [(fn() -> Runtime, usize); 5] = [
        (workload, MAXIMAL_STEPS),
        (conflicting_workload, MAXIMAL_STEPS),
        (warping_workload, WARP_STEPS),
        (dma_workload, WARP_STEPS),
        (parked_for_good, 1),
    ];
    for (build, expected) in cases {
        let mut rt = build();
        let mut steps = 0usize;
        loop {
            let predicted = rt.can_take_another_step();
            match rt.step() {
                Ok(step) => {
                    assert!(predicted, "a step ran where the predicate saw none left");
                    rt.commit_step(&step.result, &step.effects).unwrap();
                    steps += 1;
                }
                Err(e) => {
                    assert!(
                        !predicted,
                        "the predicate saw a step left and the step refused: {e}",
                    );
                    break;
                }
            }
        }
        assert_eq!(steps, expected, "the walk covered the whole workload");
    }
}

#[test]
fn the_search_at_the_exact_length_hits_no_bound() {
    let result = explore_window(
        workload,
        &ExplorationConfig {
            max_schedules: 256,
            max_steps_per_run: MAXIMAL_STEPS,
        },
    );
    assert_eq!(result.baseline_stop, StopReason::Stalled);
    assert_eq!(result.baseline_steps, MAXIMAL_STEPS);
    assert!(
        !result.bounds_hit,
        "the cap matched the workload, so nothing was cut short",
    );
    // Counting units touch nothing shared, so the baseline is the only
    // execution; the alternates meet the cap in
    // `every_alternate_at_the_exact_length_hits_no_bound`.
    assert_eq!(
        result.classes_explored,
        Some(1),
        "a search that cut nothing short can still claim its cover",
    );
    assert!(result.schedules.is_empty());
}

/// The cap over a replay, where the scheduler left installed is the
/// previous step's prescribed choice.
#[test]
fn every_alternate_at_the_exact_length_hits_no_bound() {
    let result = explore_window(
        conflicting_workload,
        &ExplorationConfig {
            max_schedules: 256,
            max_steps_per_run: MAXIMAL_STEPS,
        },
    );
    assert_eq!(result.baseline_stop, StopReason::Stalled);
    assert!(
        !result.schedules.is_empty(),
        "the stores conflict, so the search replays alternates against the cap",
    );
    for record in &result.schedules {
        assert_eq!(record.stop, StopReason::Stalled, "{record:?}");
        assert!(!record.truncated, "{record:?}");
    }
    assert!(!result.bounds_hit);
    assert_eq!(
        result.outcome,
        cellgov_explore::classify::OutcomeClass::ScheduleSensitive,
        "which unit stored last decides the memory, and every replay finished",
    );
}

#[test]
fn the_search_one_step_short_hits_the_bound() {
    let result = explore_window(
        workload,
        &ExplorationConfig {
            max_schedules: 256,
            max_steps_per_run: MAXIMAL_STEPS - 1,
        },
    );
    assert_eq!(result.baseline_stop, StopReason::StepBound);
    assert!(result.bounds_hit);
}
