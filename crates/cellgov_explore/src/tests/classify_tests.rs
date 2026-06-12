//! Construction and equality of the schedule-outcome classification types.

use super::*;

#[test]
fn outcome_class_equality() {
    assert_eq!(OutcomeClass::ScheduleStable, OutcomeClass::ScheduleStable);
    assert_ne!(
        OutcomeClass::ScheduleStable,
        OutcomeClass::ScheduleSensitive
    );
}

#[test]
fn schedule_record_holds_data() {
    let rec = ScheduleRecord {
        branch_step: 3,
        alternate_choice: UnitId::new(1),
        memory_hash: 0xDEAD,
    };
    assert_eq!(rec.branch_step, 3);
    assert_eq!(rec.memory_hash, 0xDEAD);
}

#[test]
fn exploration_result_construction() {
    let res = ExplorationResult {
        baseline_hash: 0xBEEF,
        schedules: vec![],
        outcome: OutcomeClass::ScheduleStable,
        total_branching_points: 0,
        bounds_hit: false,
        schedules_pruned: 0,
    };
    assert_eq!(res.outcome, OutcomeClass::ScheduleStable);
    assert!(!res.bounds_hit);
}
