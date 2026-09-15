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
use cellgov_event::UnitId;

fn sample_result() -> ExplorationResult {
    ExplorationResult {
        baseline_hash: 0xDEADBEEF,
        schedules: vec![ScheduleRecord {
            branch_step: 0,
            alternate_choice: UnitId::new(1),
            memory_hash: 0xCAFEBABE,
            truncated: false,
        }],
        outcome: OutcomeClass::ScheduleSensitive,
        total_branching_points: 2,
        bounds_hit: false,
        schedules_pruned: 1,
        schedules_truncated: 0,
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
    assert!(text.contains("DIVERGED"));
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
    assert_eq!(v["schedules"][0]["diverged"], true);
    assert_eq!(v["schedules"][0]["truncated"], false);
}

#[test]
fn stable_result_no_diverged_tag() {
    let r = ExplorationResult {
        baseline_hash: 0xBEEF,
        schedules: vec![],
        outcome: OutcomeClass::ScheduleStable,
        total_branching_points: 1,
        bounds_hit: false,
        schedules_pruned: 1,
        schedules_truncated: 0,
        first_invariant_break: None,
    };
    let text = format_human(&r);
    assert!(text.contains("schedule-stable"));
    assert!(!text.contains("DIVERGED"));
    assert!(!text.contains("schedules:\n"));
}

#[test]
fn a_truncated_record_is_never_labelled_diverged() {
    let r = ExplorationResult {
        baseline_hash: 0xDEADBEEF,
        schedules: vec![ScheduleRecord {
            branch_step: 0,
            alternate_choice: UnitId::new(1),
            memory_hash: 0xCAFEBABE,
            truncated: true,
        }],
        outcome: OutcomeClass::Inconclusive,
        total_branching_points: 2,
        bounds_hit: true,
        schedules_pruned: 0,
        schedules_truncated: 1,
        first_invariant_break: None,
    };
    let text = format_human(&r);
    assert!(text.contains("schedules_truncated: 1"));
    assert!(text.contains("TRUNCATED"));
    assert!(
        !text.contains("DIVERGED"),
        "a prefix hash differs from the baseline for a reason that is not divergence",
    );

    let v: serde_json::Value = serde_json::from_str(&format_json(&r)).expect("valid JSON");
    assert_eq!(v["schedules"][0]["diverged"], false);
    assert_eq!(v["schedules"][0]["truncated"], true);
    assert_eq!(v["schedules_truncated"], 1);
}
