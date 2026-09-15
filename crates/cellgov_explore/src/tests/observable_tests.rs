//! Every report names the observable its verdict is read against.

use crate::classify::{
    ExplorationResult, OutcomeClass, ScheduleRecord, OBSERVABLE, OBSERVABLE_LABEL,
};
use crate::report::{format_human, format_json};
use crate::util::StopReason;
use cellgov_event::UnitId;

fn result(outcome: OutcomeClass) -> ExplorationResult {
    ExplorationResult {
        baseline_hash: 0x1234,
        baseline_steps: 3,
        baseline_stop: StopReason::Stalled,
        schedules: vec![ScheduleRecord {
            branch_step: 1,
            alternate_choice: UnitId::new(1),
            memory_hash: 0x1234,
            stop: StopReason::Stalled,
            truncated: false,
        }],
        outcome,
        total_branching_points: 1,
        classes_explored: Some(2),
        reversals_dropped: 0,
        bounds_hit: false,
        schedules_pruned: 0,
        schedules_truncated: 0,
        schedules_refused: 0,
        first_invariant_break: None,
    }
}

#[test]
fn the_human_report_names_the_observable_beside_the_outcome() {
    for outcome in [
        OutcomeClass::ScheduleStable,
        OutcomeClass::ScheduleSensitive,
        OutcomeClass::Inconclusive,
    ] {
        let out = format_human(&result(outcome));
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].starts_with("outcome: "), "{out}");
        assert_eq!(lines[1], format!("observable: {OBSERVABLE}"), "{out}");
    }
}

#[test]
fn the_json_report_carries_the_observable_label() {
    let out = format_json(&result(OutcomeClass::ScheduleStable));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["observable"], OBSERVABLE_LABEL);
}

/// The sentence names all of memory and the end of the run: a reader
/// who sees it cannot take the verdict for one over declared regions.
#[test]
fn the_observable_names_every_address_space_and_the_end_of_the_run() {
    assert!(OBSERVABLE.contains("every address space"), "{OBSERVABLE}");
    assert!(OBSERVABLE.contains("end of the run"), "{OBSERVABLE}");
    assert!(!OBSERVABLE.contains("region"), "{OBSERVABLE}");
}
