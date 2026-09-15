//! A prefix hash never reads as a divergence.
//!
//! Four pieces carry that rule, and each case below fails when one of
//! them alone stops working: [`StopReason::is_truncated`], the
//! [`ScheduleRecord::truncated`] flag `for_each_alternate` sets,
//! [`AlternateIteration::mark_baseline_truncated`], and the `DIVERGED`
//! suppression both report formatters apply.

use crate::classify::{BaselineRun, ExplorationResult, OutcomeClass, ScheduleRecord};
use crate::config::ExplorationConfig;
use crate::decision::DecisionLog;
use crate::report::{format_human, format_json};
use crate::util::{classify_iteration, for_each_alternate, AlternateIteration, StopReason};
use cellgov_core::{CommitError, Runtime, StepError};
use cellgov_event::UnitId;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

const BASELINE_HASH: u64 = 0x1111_1111_1111_1111;

/// A hash that differs from [`BASELINE_HASH`], so a record carrying it
/// reads as a divergence wherever nothing withdraws it.
const OTHER_HASH: u64 = 0x2222_2222_2222_2222;

/// One refused commit, which stands for every shape the pipeline gives.
const REFUSED_COMMIT: CommitError = CommitError::OutOfRange { effect_index: 0 };

/// Every stop reason that leaves a prefix of the schedule.
const EVERY_TRUNCATING_REASON: [StopReason; 7] = [
    StopReason::StepBound,
    StopReason::StepError(StepError::NoRunnableUnit),
    StopReason::StepError(StepError::AllBlocked),
    StopReason::StepError(StepError::MaxStepsExceeded),
    StopReason::StepError(StepError::TimeOverflow),
    StopReason::StepError(StepError::SchedulerNotReinstalled),
    StopReason::CommitError(REFUSED_COMMIT),
];

/// `count` units that all write the same word, so dependency pruning
/// keeps every alternate and each case has records to read.
fn contending_log(count: u32) -> DecisionLog {
    let mem = GuestMemory::new(64);
    let mut rt = Runtime::new(mem, Budget::new(100), 100);
    for imm in 0..count {
        rt.register_unit_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0xAA + imm),
                    FakeOp::SharedStore { addr: 0, len: 4 },
                    FakeOp::End,
                ],
            )
        });
    }
    let (log, _) = crate::observer::observe_decisions(&mut rt);
    log
}

/// A baseline that ran the workload out, which every alternate below is
/// measured against.
const COMPLETE_BASELINE: BaselineRun = BaselineRun {
    hash: BASELINE_HASH,
    steps: 6,
    stop: StopReason::Stalled,
};

#[test]
fn no_stop_short_of_a_stall_can_contribute_a_divergence() {
    let log = contending_log(2);
    let config = ExplorationConfig::default();

    for reason in EVERY_TRUNCATING_REASON {
        // Each replay reports a hash the baseline did not, which is
        // what a divergence looks like when the run finished.
        let iter = for_each_alternate(&log, &config, BASELINE_HASH, |_, _| (OTHER_HASH, reason));
        assert!(
            !iter.schedules.is_empty(),
            "{reason}: the workload must offer an alternate to record"
        );
        assert!(
            !iter.found_divergence,
            "{reason}: a prefix hash differs from a finished baseline whether or not \
             the workload is schedule-sensitive"
        );
        assert!(iter.bounds_hit, "{reason}");
        assert_eq!(iter.schedules_truncated, iter.schedules.len(), "{reason}");
        assert!(
            iter.schedules.iter().all(|s| s.truncated),
            "{reason}: every record carries the flag its report reads"
        );
    }

    let stalled = for_each_alternate(&log, &config, BASELINE_HASH, |_, _| {
        (OTHER_HASH, StopReason::Stalled)
    });
    assert!(
        stalled.found_divergence,
        "a run that finished on a hash of its own is the divergence the cases above withhold"
    );
    assert!(stalled.schedules.iter().all(|s| !s.truncated));
    assert_eq!(stalled.schedules_truncated, 0);
}

#[test]
fn a_truncated_alternate_against_a_finished_baseline_is_inconclusive() {
    let log = contending_log(2);
    let iter = for_each_alternate(
        &log,
        &ExplorationConfig::default(),
        BASELINE_HASH,
        |_, _| (OTHER_HASH, StopReason::StepBound),
    );
    assert!(!iter.schedules.is_empty());

    let result = classify_iteration(iter, COMPLETE_BASELINE, log.branching_count(), None);
    assert!(result.schedules.iter().all(|s| s.truncated));
    assert_eq!(result.outcome, OutcomeClass::Inconclusive);
    assert!(result.bounds_hit);
}

#[test]
fn a_baseline_that_stopped_short_withdraws_a_record_it_had_already_cleared() {
    // Each record is written while the baseline still reads as
    // finished, and the baseline's own stop is applied after the last
    // one. A record cleared earlier is withdrawn with the rest.
    let log = contending_log(3);
    let mut iter = for_each_alternate(
        &log,
        &ExplorationConfig::default(),
        BASELINE_HASH,
        |_, _| (OTHER_HASH, StopReason::Stalled),
    );
    assert!(
        iter.schedules.len() > 1,
        "the case needs more than one record for the earlier one to be at stake"
    );
    assert!(iter.found_divergence);
    assert!(iter.schedules.iter().all(|s| !s.truncated));

    iter.mark_baseline_truncated();
    assert!(
        iter.schedules.iter().all(|s| s.truncated),
        "a prefix baseline withdraws every record measured against it"
    );
    assert!(!iter.found_divergence);
    assert!(iter.bounds_hit);

    let truncated_baseline = BaselineRun {
        stop: StopReason::StepBound,
        ..COMPLETE_BASELINE
    };
    assert_eq!(
        classify_iteration(iter, truncated_baseline, log.branching_count(), None).outcome,
        OutcomeClass::Inconclusive,
    );
}

#[test]
fn a_bound_hit_with_every_explored_schedule_in_agreement_is_still_inconclusive() {
    // The claim that an exploration ruled divergence out holds only
    // over a complete enumeration. Three contending units offer more
    // alternates than this cap admits.
    let log = contending_log(3);
    let config = ExplorationConfig {
        max_schedules: 1,
        ..ExplorationConfig::default()
    };
    let iter = for_each_alternate(&log, &config, BASELINE_HASH, |_, _| {
        (BASELINE_HASH, StopReason::Stalled)
    });
    assert_eq!(iter.schedules.len(), 1, "the cap must cut the enumeration");
    assert!(!iter.found_divergence);
    assert_eq!(iter.schedules_truncated, 0);
    assert!(
        iter.schedules.iter().all(|s| !s.truncated),
        "the cap stopped the enumeration, not any replay"
    );
    assert!(iter.bounds_hit);

    let result = classify_iteration(iter, COMPLETE_BASELINE, log.branching_count(), None);
    assert_eq!(
        result.outcome,
        OutcomeClass::Inconclusive,
        "every schedule the run reached agreed, and the ones it never reached decide the verdict",
    );
}

/// A result holding one record whose hash differs from the baseline's,
/// withdrawn by `truncated_by`.
fn withdrawn_record(replay_stop: StopReason) -> ExplorationResult {
    ExplorationResult {
        baseline_hash: BASELINE_HASH,
        baseline_steps: 6,
        baseline_stop: StopReason::Stalled,
        schedules: vec![ScheduleRecord {
            branch_step: 0,
            alternate_choice: UnitId::new(1),
            memory_hash: OTHER_HASH,
            stop: replay_stop,
            truncated: true,
        }],
        outcome: OutcomeClass::Inconclusive,
        total_branching_points: 1,
        bounds_hit: true,
        schedules_pruned: 0,
        schedules_truncated: 1,
        schedules_refused: 0,
        first_invariant_break: None,
    }
}

#[test]
fn neither_formatter_calls_a_withdrawn_record_a_divergence() {
    // One record per thing that can withdraw it: the replay's own stop,
    // and a baseline whose stop the record cannot report.
    for (replay_stop, by) in [
        (StopReason::StepBound, "replay"),
        (StopReason::Stalled, "baseline"),
    ] {
        let result = withdrawn_record(replay_stop);
        let text = format_human(&result);
        assert!(
            text.contains(&format!("TRUNCATED({by})")),
            "the row must name what withdrew it: {text}"
        );
        assert!(
            !text.contains("DIVERGED"),
            "the hash differs from the baseline's for a reason that is not divergence: {text}"
        );

        let v: serde_json::Value = serde_json::from_str(&format_json(&result)).expect("valid JSON");
        assert_eq!(v["schedules"][0]["diverged"], false, "{by}");
        assert_eq!(v["schedules"][0]["truncated"], true, "{by}");
        assert_eq!(v["schedules"][0]["truncated_by"], by);
        assert_ne!(
            v["schedules"][0]["memory_hash"], v["baseline_hash"],
            "{by}: a record whose hash matched the baseline's would pass the case unread",
        );
    }
}

#[test]
fn a_finished_record_that_disagrees_is_the_divergence_the_others_withhold() {
    // Positive control over the formatters: without it, a suppression
    // that labelled nothing DIVERGED would pass every case above.
    let result = ExplorationResult {
        schedules: vec![ScheduleRecord {
            truncated: false,
            stop: StopReason::Stalled,
            ..withdrawn_record(StopReason::Stalled).schedules[0].clone()
        }],
        outcome: OutcomeClass::ScheduleSensitive,
        bounds_hit: false,
        schedules_truncated: 0,
        ..withdrawn_record(StopReason::Stalled)
    };

    let text = format_human(&result);
    assert!(text.contains("DIVERGED"), "{text}");
    assert!(!text.contains("TRUNCATED"), "{text}");

    let v: serde_json::Value = serde_json::from_str(&format_json(&result)).expect("valid JSON");
    assert_eq!(v["schedules"][0]["diverged"], true);
    assert_eq!(v["schedules"][0]["truncated"], false);
    assert_eq!(v["schedules"][0]["truncated_by"], serde_json::Value::Null);
}

#[test]
fn a_tally_that_reports_no_bound_and_no_divergence_is_the_one_stable_reading() {
    // The floor under every case above. `classify_iteration` reaches
    // `ScheduleStable` where nothing was withdrawn and no bound was
    // hit. An assertion of `Inconclusive` elsewhere is therefore a
    // claim about the rule, not about the classifier.
    let clean = AlternateIteration {
        schedules: vec![ScheduleRecord {
            branch_step: 0,
            alternate_choice: UnitId::new(1),
            memory_hash: BASELINE_HASH,
            stop: StopReason::Stalled,
            truncated: false,
        }],
        bounds_hit: false,
        found_divergence: false,
        schedules_pruned: 0,
        schedules_truncated: 0,
        schedules_refused: 0,
    };
    assert_eq!(
        classify_iteration(clean, COMPLETE_BASELINE, 1, None).outcome,
        OutcomeClass::ScheduleStable,
    );
}
