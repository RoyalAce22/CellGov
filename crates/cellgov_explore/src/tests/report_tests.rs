//! Exploration-report rendering: outcome-label distinctness plus human and JSON output shape.

use super::*;
use strum::VariantArray;

#[test]
fn outcome_label_is_total_and_distinct() {
    let labels: Vec<&'static str> = OutcomeClass::VARIANTS
        .iter()
        .map(|c| outcome_label(*c))
        .collect();
    for (i, a) in labels.iter().enumerate() {
        for (j, b) in labels.iter().enumerate() {
            if i != j {
                assert_ne!(a, b, "outcome_label not distinct at {i}/{j}");
            }
        }
    }
}
use crate::classify::{ExplorationResult, OutcomeClass, ScheduleRecord};
use crate::util::StopReason;
use cellgov_core::{CommitError, StepError};
use cellgov_event::UnitId;

/// One refused commit, which stands for every shape the pipeline gives.
const REFUSED_COMMIT: CommitError = CommitError::OutOfRange { effect_index: 0 };

fn sample_result() -> ExplorationResult {
    ExplorationResult {
        baseline_hash: 0xDEADBEEF,
        baseline_steps: 12,
        baseline_stop: StopReason::Stalled,
        schedules: vec![ScheduleRecord {
            branch_step: 0,
            alternate_choice: UnitId::new(1),
            memory_hash: 0xCAFEBABE,
            stop: StopReason::Stalled,
            truncated: false,
        }],
        outcome: OutcomeClass::ScheduleSensitive,
        total_branching_points: 2,
        classes_explored: None,
        bounds_hit: false,
        schedules_pruned: 1,
        schedules_truncated: 0,
        schedules_refused: 0,
        first_invariant_break: None,
    }
}

#[test]
fn human_report_contains_key_fields() {
    let text = format_human(&sample_result());
    assert!(text.contains("schedule-sensitive"));
    assert!(text.contains("0x00000000deadbeef"));
    assert!(text.contains("branching_points: 2"));
    assert!(text.contains("schedules_explored: 1"));
    assert!(text.contains("schedules_pruned: 1"));
    assert!(text.contains("schedules_truncated: 0"));
    assert!(text.contains("baseline_steps: 12"));
    assert!(text.contains("baseline_stop: stalled (finished)"));
    assert!(text.contains("DIVERGED"));
}

#[test]
fn a_bound_and_a_refusal_read_apart_in_both_reports() {
    let bounded = ExplorationResult {
        baseline_stop: StopReason::StepError(StepError::MaxStepsExceeded),
        ..sample_result()
    };
    let refused = ExplorationResult {
        baseline_stop: StopReason::CommitError(REFUSED_COMMIT),
        schedules_refused: 3,
        ..sample_result()
    };

    assert!(format_human(&bounded).contains("(bound)"));
    assert!(format_human(&refused).contains("commit refused: "));
    assert!(format_human(&refused).contains("(refusal)"));
    assert!(format_human(&refused).contains("schedules_refused: 3"));

    let b: serde_json::Value = serde_json::from_str(&format_json(&bounded)).expect("valid JSON");
    let r: serde_json::Value = serde_json::from_str(&format_json(&refused)).expect("valid JSON");
    assert_eq!(b["baseline_stop_class"], "bound");
    assert_eq!(b["schedules_refused"], 0);
    assert_eq!(r["baseline_stop_class"], "refusal");
    assert_eq!(r["schedules_refused"], 3);
}

#[test]
fn json_report_parses_correctly() {
    let json_str = format_json(&sample_result());
    let v: serde_json::Value = serde_json::from_str(&json_str).expect("valid JSON");
    assert_eq!(v["outcome"], "schedule-sensitive");
    assert_eq!(v["branching_points"], 2);
    assert_eq!(v["schedules_explored"], 1);
    assert_eq!(v["schedules_pruned"], 1);
    assert_eq!(v["schedules_truncated"], 0);
    assert_eq!(v["baseline_steps"], 12);
    assert_eq!(v["baseline_stop"], "stalled");
    assert_eq!(v["schedules"][0]["diverged"], true);
    assert_eq!(v["schedules"][0]["truncated"], false);
    assert_eq!(v["schedules"][0]["truncated_by"], serde_json::Value::Null);
}

#[test]
fn both_reports_say_whether_the_search_covered_every_class() {
    // Absence is a reading of its own: a verdict backed by no class
    // count answers for the schedules the search sampled, and an empty
    // row is how a reader sees that.
    let covered = ExplorationResult {
        classes_explored: Some(6),
        ..sample_result()
    };
    assert!(
        format_human(&covered).contains("classes_explored: 6"),
        "{}",
        format_human(&covered)
    );
    let v: serde_json::Value = serde_json::from_str(&format_json(&covered)).expect("valid JSON");
    assert_eq!(v["classes_explored"], 6);

    let uncovered = sample_result();
    assert!(
        format_human(&uncovered).contains("classes_explored: not covered"),
        "{}",
        format_human(&uncovered)
    );
    let v: serde_json::Value = serde_json::from_str(&format_json(&uncovered)).expect("valid JSON");
    assert_eq!(v["classes_explored"], serde_json::Value::Null);
}

#[test]
fn a_capped_replay_row_reads_as_a_bound_and_not_as_a_refusal() {
    let r = ExplorationResult {
        schedules: vec![ScheduleRecord {
            branch_step: 0,
            alternate_choice: UnitId::new(1),
            memory_hash: 0xCAFEBABE,
            stop: StopReason::StepError(StepError::MaxStepsExceeded),
            truncated: true,
        }],
        outcome: OutcomeClass::Inconclusive,
        bounds_hit: true,
        schedules_truncated: 1,
        ..sample_result()
    };

    let text = format_human(&r);
    assert!(
        text.contains("stop=step refused: max-steps cap exceeded (bound)"),
        "the stop wording calls a cap a refusal, so the class rides along: {text}"
    );

    let v: serde_json::Value = serde_json::from_str(&format_json(&r)).expect("valid JSON");
    assert_eq!(v["schedules"][0]["stop_class"], "bound");
    assert_eq!(v["schedules_refused"], 0);
}

#[test]
fn a_record_the_baseline_withdrew_does_not_read_as_a_replay_that_stopped_short() {
    // What `AlternateIteration::mark_baseline_truncated` leaves behind:
    // the flag set on a record whose own replay ran the workload out.
    let r = ExplorationResult {
        baseline_stop: StopReason::StepBound,
        schedules: vec![ScheduleRecord {
            branch_step: 0,
            alternate_choice: UnitId::new(1),
            memory_hash: 0xCAFEBABE,
            stop: StopReason::Stalled,
            truncated: true,
        }],
        outcome: OutcomeClass::Inconclusive,
        bounds_hit: true,
        schedules_truncated: 1,
        ..sample_result()
    };

    let text = format_human(&r);
    assert!(
        text.contains("TRUNCATED(baseline)"),
        "the row must name what withdrew it: {text}"
    );
    assert!(
        !text.contains("TRUNCATED(replay)"),
        "this replay reached a stall: {text}"
    );

    let v: serde_json::Value = serde_json::from_str(&format_json(&r)).expect("valid JSON");
    assert_eq!(v["schedules"][0]["truncated"], true);
    assert_eq!(v["schedules"][0]["truncated_by"], "baseline");
    assert_eq!(
        v["schedules"][0]["stop_class"], "finished",
        "the stop is the replay's own, and the report says so beside what withdrew it",
    );
}

#[test]
fn stable_result_no_diverged_tag() {
    let r = ExplorationResult {
        baseline_hash: 0xBEEF,
        schedules: vec![],
        outcome: OutcomeClass::ScheduleStable,
        total_branching_points: 1,
        classes_explored: None,
        schedules_pruned: 1,
        ..sample_result()
    };
    let text = format_human(&r);
    assert!(text.contains("schedule-stable"));
    assert!(!text.contains("DIVERGED"));
    assert!(!text.contains("schedules:\n"));
}

#[test]
fn a_truncated_record_is_never_labelled_diverged() {
    let r = ExplorationResult {
        schedules: vec![ScheduleRecord {
            branch_step: 0,
            alternate_choice: UnitId::new(1),
            memory_hash: 0xCAFEBABE,
            stop: StopReason::StepBound,
            truncated: true,
        }],
        outcome: OutcomeClass::Inconclusive,
        bounds_hit: true,
        schedules_pruned: 0,
        schedules_truncated: 1,
        ..sample_result()
    };
    let text = format_human(&r);
    assert!(text.contains("schedules_truncated: 1"));
    assert!(text.contains("TRUNCATED(replay)"));
    assert!(
        text.contains("stop=replay step bound reached (bound)"),
        "a truncated row must say what cut it short: {text}"
    );
    assert!(
        !text.contains("DIVERGED"),
        "a prefix hash differs from the baseline for a reason that is not divergence",
    );

    let v: serde_json::Value = serde_json::from_str(&format_json(&r)).expect("valid JSON");
    assert_eq!(v["schedules"][0]["diverged"], false);
    assert_eq!(v["schedules"][0]["truncated"], true);
    assert_eq!(v["schedules"][0]["stop_class"], "bound");
    assert_eq!(v["schedules"][0]["truncated_by"], "replay");
    assert_eq!(v["schedules_truncated"], 1);
}
