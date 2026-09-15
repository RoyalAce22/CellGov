//! A prefix hash never reads as a divergence.
//!
//! Three pieces carry that rule, and each case below fails when one of
//! them alone stops working:
//!
//! - [`StopReason::is_truncated`];
//! - [`AlternateIteration::mark_baseline_truncated`];
//! - the `DIVERGED` suppression both report formatters apply.
//!
//! The [`ScheduleRecord::truncated`] flag each search sets from
//! `is_truncated` rides along; the cases build it by hand.

use crate::classify::{BaselineRun, ExplorationResult, OutcomeClass, ScheduleRecord};
use crate::config::ExplorationConfig;
use crate::optimal::explore_optimal;
use crate::report::{format_human, format_json};
use crate::util::{classify_iteration, AlternateIteration, StopClass, StopReason};
use cellgov_core::{CommitError, Runtime, StepError};
use cellgov_effects::FaultKind;
use cellgov_event::UnitId;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp, FAKE_FAULT_CODE};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;
use strum::VariantArray;

const BASELINE_HASH: u64 = 0x1111_1111_1111_1111;

/// A hash that differs from [`BASELINE_HASH`], so a record carrying it
/// reads as a divergence wherever nothing withdraws it.
const OTHER_HASH: u64 = 0x2222_2222_2222_2222;

/// One refused commit, which stands for every shape the pipeline
/// gives. `a_refusal_of_any_shape_truncates` is what earns that.
const REFUSED_COMMIT: CommitError = CommitError::OutOfRange { effect_index: 0 };

/// Every stop reason that leaves a prefix of the schedule.
///
/// The step-refusal half comes from `StepError::VARIANTS` rather than
/// a list here, so a variant added to the runtime reaches this sweep
/// without anyone widening anything.
fn every_truncating_reason() -> Vec<StopReason> {
    std::iter::once(StopReason::StepBound)
        .chain(
            StepError::VARIANTS
                .iter()
                .copied()
                .map(StopReason::StepError),
        )
        .chain(std::iter::once(StopReason::CommitError(REFUSED_COMMIT)))
        .chain(std::iter::once(StopReason::Faulted(FaultKind::Guest(
            FAKE_FAULT_CODE,
        ))))
        .collect()
}

/// The two stops that end a maximal execution.
///
/// A run that reaches either one answers for its whole self: its hash
/// stands for the run, and the search reads its races.
const MAXIMAL_STOPS: [StopReason; 2] = [StopReason::Stalled, StopReason::Deadlocked];

/// `count` units that all write the same word, so every pair of them
/// conflicts and a search has alternates to record.
fn contending_runtime(count: u32) -> Runtime {
    let mem = GuestMemory::new(64);
    let mut rt = Runtime::new(mem, Budget::new(1), 100);
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
    rt
}

/// The tally a search keeps for `count` alternates that each committed
/// `hash` and stopped for `reason`.
///
/// Every search builds this shape and hands it to
/// [`classify_iteration`], so the rules below are the ones the live
/// searches run.
fn tally(count: usize, hash: u64, reason: StopReason) -> AlternateIteration {
    let truncated = reason.is_truncated();
    let schedules: Vec<ScheduleRecord> = (0..count)
        .map(|index| ScheduleRecord {
            branch_step: index,
            alternate_choice: UnitId::new(index as u64),
            memory_hash: hash,
            stop: reason,
            truncated,
        })
        .collect();
    AlternateIteration {
        found_divergence: !truncated && hash != BASELINE_HASH,
        bounds_hit: truncated,
        schedules_truncated: if truncated { schedules.len() } else { 0 },
        schedules_refused: if reason.class() == StopClass::Refusal {
            schedules.len()
        } else {
            0
        },
        schedules_pruned: 0,
        schedules,
    }
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
    for reason in every_truncating_reason() {
        // Each replay reports a hash the baseline did not, which is
        // what a divergence looks like when the run finished.
        let iter = tally(2, OTHER_HASH, reason);
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

    for reason in MAXIMAL_STOPS {
        let maximal = tally(2, OTHER_HASH, reason);
        assert!(
            maximal.found_divergence,
            "{reason}: a run that ended on a hash of its own is the divergence the cases \
             above withhold"
        );
        assert!(maximal.schedules.iter().all(|s| !s.truncated), "{reason}");
        assert_eq!(maximal.schedules_truncated, 0, "{reason}");
    }
}

#[test]
fn a_truncated_alternate_against_a_finished_baseline_is_inconclusive() {
    let iter = tally(1, OTHER_HASH, StopReason::StepBound);
    assert!(!iter.schedules.is_empty());

    let result = classify_iteration(iter, COMPLETE_BASELINE, 1, None);
    assert!(result.schedules.iter().all(|s| s.truncated));
    assert_eq!(result.outcome, OutcomeClass::Inconclusive);
    assert!(result.bounds_hit);
}

#[test]
fn a_baseline_that_stopped_short_withdraws_a_record_it_had_already_cleared() {
    // Each record is written while the baseline still reads as
    // finished, and the baseline's own stop is applied after the last
    // one. A record cleared earlier is withdrawn with the rest.
    let mut iter = tally(2, OTHER_HASH, StopReason::Stalled);
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
        classify_iteration(iter, truncated_baseline, 2, None).outcome,
        OutcomeClass::Inconclusive,
    );
}

#[test]
fn a_bound_hit_with_every_explored_schedule_in_agreement_is_still_inconclusive() {
    // The claim that an exploration ruled divergence out holds only
    // over a complete enumeration. Three contending units offer more
    // alternates than this cap admits.
    let mut iter = tally(1, BASELINE_HASH, StopReason::Stalled);
    // The cap stopped the search, not any replay.
    iter.bounds_hit = true;
    assert_eq!(iter.schedules.len(), 1, "the cap must cut the search");
    assert!(!iter.found_divergence);
    assert_eq!(iter.schedules_truncated, 0);
    assert!(
        iter.schedules.iter().all(|s| !s.truncated),
        "the cap stopped the enumeration, not any replay"
    );
    assert!(iter.bounds_hit);

    let result = classify_iteration(iter, COMPLETE_BASELINE, 3, None);
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
        classes_explored: None,
        reversals_dropped: 0,
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

/// The rules above, over the live search rather than a tally built by
/// hand: a cap stops the baseline, and the run claims nothing.
#[test]
fn a_capped_search_over_a_contending_workload_claims_nothing() {
    let config = ExplorationConfig {
        max_schedules: 256,
        max_steps_per_run: 2,
    };
    let result = explore_optimal(|| contending_runtime(2), &config);
    assert!(result.baseline_stop.is_truncated());
    assert!(result.bounds_hit);
    assert_eq!(
        result.classes_explored, None,
        "a prefix baseline covers no class",
    );
    assert_eq!(result.outcome, OutcomeClass::Inconclusive);
    // The search reads no race off a prefix execution, so it owes no
    // reversal and records nothing.
    assert!(
        result.schedules.is_empty(),
        "a prefix baseline's races cover a prefix, so the search owes no reversal",
    );
    // With nothing owed nothing is dropped, so the empty count above is
    // the cap and not a reversal this run gave up.
    assert_eq!(
        result.reversals_dropped, 0,
        "a prefix execution names no race, so no branch is there to drop",
    );
    assert_eq!(result.schedules_truncated, 0);
}

/// The sweep runs one case per way a run stops short, and the two
/// maximal stops are the ones it does not cover.
///
/// The width and the repeat check are what keep the sweep from going
/// vacuous: a hand-written list put back in place of the
/// `StepError::VARIANTS` chain goes red here the moment it is a variant
/// short or names one twice. The truncation claims go red on a
/// [`StopReason::is_truncated`] that admits a maximal stop, or that
/// refuses one of the stops the sweep covers.
#[test]
fn the_sweep_runs_one_case_per_way_a_step_can_refuse() {
    let reasons = every_truncating_reason();
    assert_eq!(
        reasons.len(),
        StepError::VARIANTS.len() + 3,
        "the sweep must run one case per step refusal, plus the replay bound, a \
         refused commit and a fault",
    );
    for (index, reason) in reasons.iter().enumerate() {
        assert!(
            !reasons[..index].contains(reason),
            "{reason}: the sweep runs this case twice, so it is one case short elsewhere",
        );
        assert!(reason.is_truncated(), "{reason}");
    }
    for reason in MAXIMAL_STOPS {
        assert!(!reason.is_truncated(), "{reason}");
        assert!(
            !reasons.contains(&reason),
            "{reason}: a maximal stop cannot also be one the sweep truncates",
        );
    }
}

/// A refused commit truncates whatever shape it took, which is what
/// lets one stand in for every other in the sweep above.
#[test]
fn a_refusal_of_any_shape_truncates() {
    let shapes = [
        CommitError::PayloadLengthMismatch { effect_index: 0 },
        CommitError::OutOfRange { effect_index: 0 },
        CommitError::UnknownMailbox {
            effect_index: 0,
            mailbox: cellgov_sync::MailboxId::new(0),
        },
        CommitError::UnknownSignal {
            effect_index: 0,
            signal: cellgov_sync::SignalId::new(0),
        },
        CommitError::UnknownWakeTarget {
            effect_index: 0,
            target: UnitId::new(0),
        },
        CommitError::UnknownSourceUnit {
            effect_index: 0,
            source_unit: UnitId::new(0),
        },
        CommitError::DmaDestinationOutOfRange { effect_index: 0 },
        CommitError::DmaDestinationReserved {
            effect_index: 0,
            addr: 0,
            region: "reserved",
        },
        CommitError::Memory(cellgov_mem::MemError::LengthMismatch),
    ];
    // The count stands in for exhaustiveness only over a list that
    // names each shape once: a repeat would hold the length up while a
    // shape the pipeline can give went unlisted.
    for (index, shape) in shapes.iter().enumerate() {
        assert!(
            !shapes[..index].contains(shape),
            "{shape}: listed twice, so a shape the pipeline can give is missing",
        );
    }
    assert_eq!(
        shapes.len(),
        <CommitError as strum::EnumCount>::COUNT,
        "a commit refusal the pipeline can give is missing from this list",
    );
    for shape in shapes {
        let reason = StopReason::CommitError(shape);
        assert!(reason.is_truncated(), "{shape}");
        assert_eq!(reason.class(), crate::util::StopClass::Refusal, "{shape}");
    }
}
