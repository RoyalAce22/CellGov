//! The stop-reason classification every exploration report reads.

use super::*;
use cellgov_core::{CommitError, StepError};
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;
use strum::VariantArray;

/// One refused commit, which stands for every shape the pipeline gives.
const REFUSED_COMMIT: CommitError = CommitError::OutOfRange { effect_index: 0 };

/// Every [`StepError`] the runtime can give.
const EVERY_STEP_ERROR: [StepError; 5] = [
    StepError::NoRunnableUnit,
    StepError::AllBlocked,
    StepError::MaxStepsExceeded,
    StepError::TimeOverflow,
    StepError::SchedulerNotReinstalled,
];

#[test]
fn only_a_stall_reads_as_a_finished_run() {
    for reason in EVERY_STEP_ERROR.map(StopReason::StepError) {
        assert_ne!(reason.class(), StopClass::Finished, "{reason}");
        assert!(reason.is_truncated(), "{reason}");
    }
    assert_ne!(StopReason::StepBound.class(), StopClass::Finished);
    assert_ne!(
        StopReason::CommitError(REFUSED_COMMIT).class(),
        StopClass::Finished
    );
    assert_eq!(StopReason::Stalled.class(), StopClass::Finished);
    assert!(!StopReason::Stalled.is_truncated());
}

#[test]
fn a_cap_the_caller_set_is_a_bound_and_not_a_refusal() {
    // Both caps are the caller's own: the exploration's per-replay
    // cap, and the runtime's step cap.
    assert_eq!(StopReason::StepBound.class(), StopClass::Bound);
    assert_eq!(
        StopReason::StepError(StepError::MaxStepsExceeded).class(),
        StopClass::Bound
    );
}

#[test]
fn a_refused_commit_or_step_is_a_refusal() {
    assert_eq!(
        StopReason::CommitError(REFUSED_COMMIT).class(),
        StopClass::Refusal
    );
    assert_eq!(
        StopReason::StepError(StepError::SchedulerNotReinstalled).class(),
        StopClass::Refusal
    );
}

#[test]
fn the_guest_time_ceiling_is_a_refusal_rather_than_a_cap_to_raise() {
    // `max_steps_per_run` and the runtime's step cap are the caller's
    // to widen; u64::MAX guest ticks is not, so reading it as a bound
    // sends a reader after a ceiling no flag reaches.
    assert_eq!(
        StopReason::StepError(StepError::TimeOverflow).class(),
        StopClass::Refusal
    );
}

#[test]
fn a_run_with_nothing_to_schedule_is_blocked() {
    assert_eq!(
        StopReason::StepError(StepError::AllBlocked).class(),
        StopClass::Blocked
    );
    assert_eq!(
        StopReason::StepError(StepError::NoRunnableUnit).class(),
        StopClass::Blocked
    );
}

/// Two units that write the same word, so dependency pruning keeps
/// every alternate and the tallies below have something to count.
fn contending_runtime() -> Runtime {
    let mem = GuestMemory::new(64);
    let mut rt = Runtime::new(mem, Budget::new(100), 100);
    for imm in [0xAAu32, 0xBB] {
        rt.register_unit_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(imm),
                    FakeOp::SharedStore { addr: 0, len: 4 },
                    FakeOp::End,
                ],
            )
        });
    }
    rt
}

fn contending_log() -> DecisionLog {
    let mut rt = contending_runtime();
    let (log, _) = crate::observer::observe_decisions(&mut rt);
    log
}

#[test]
fn a_refused_replay_is_counted_apart_from_one_a_cap_cut_short() {
    let log = contending_log();
    let config = ExplorationConfig::default();

    let mut alternates = 0usize;
    let refused = for_each_alternate(&log, &config, 0, |_, _| {
        alternates += 1;
        (0, StopReason::CommitError(REFUSED_COMMIT))
    });
    assert!(alternates > 0, "the workload must offer an alternate");
    assert_eq!(refused.schedules_refused, alternates);
    assert_eq!(refused.schedules_truncated, alternates);

    let bounded = for_each_alternate(&log, &config, 0, |_, _| (0, StopReason::StepBound));
    assert_eq!(bounded.schedules_truncated, alternates);
    assert_eq!(
        bounded.schedules_refused, 0,
        "a cap the caller set is not a refusal the model gave"
    );

    let stalled = for_each_alternate(&log, &config, 0, |_, _| (0, StopReason::Stalled));
    assert_eq!(stalled.schedules_truncated, 0);
    assert_eq!(stalled.schedules_refused, 0);
}

#[test]
fn a_withdrawn_baseline_taints_every_record_without_inventing_a_refusal() {
    let log = contending_log();
    let mut iter = for_each_alternate(&log, &ExplorationConfig::default(), 0, |_, _| {
        (0, StopReason::Stalled)
    });
    assert!(!iter.schedules.is_empty());

    iter.mark_baseline_truncated();
    assert_eq!(iter.schedules_truncated, iter.schedules.len());
    assert_eq!(
        iter.schedules_refused, 0,
        "the baseline's own stop is reported apart from the alternates'"
    );
    assert!(
        iter.schedules.iter().all(|s| s.stop == StopReason::Stalled),
        "a withdrawn record still reports what its own replay did",
    );
}

#[test]
fn a_baseline_that_committed_no_step_measured_nothing() {
    // Every tally is empty and the run hit no bound, so the arithmetic
    // alone reads as stable. An empty window checked nothing.
    let empty = AlternateIteration {
        schedules: Vec::new(),
        bounds_hit: false,
        found_divergence: false,
        schedules_pruned: 0,
        schedules_truncated: 0,
        schedules_refused: 0,
    };
    let ran_nothing = BaselineRun {
        hash: 0,
        steps: 0,
        stop: StopReason::Stalled,
    };
    assert_eq!(
        classify_iteration(empty, ran_nothing, 0, None).outcome,
        OutcomeClass::Inconclusive,
    );

    let one_step = BaselineRun {
        steps: 1,
        ..ran_nothing
    };
    let same_tally = AlternateIteration {
        schedules: Vec::new(),
        bounds_hit: false,
        found_divergence: false,
        schedules_pruned: 0,
        schedules_truncated: 0,
        schedules_refused: 0,
    };
    assert_eq!(
        classify_iteration(same_tally, one_step, 0, None).outcome,
        OutcomeClass::ScheduleStable,
        "one committed step is a window that ran, however little it held",
    );
}

#[test]
fn every_class_label_is_distinct() {
    let labels: Vec<&'static str> = StopClass::VARIANTS.iter().map(|c| c.label()).collect();
    let mut sorted = labels.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), labels.len(), "{labels:?}");
}
