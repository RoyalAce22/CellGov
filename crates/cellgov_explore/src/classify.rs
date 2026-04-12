//! Outcome classification for schedule exploration.
//!
//! After exploring multiple legal schedules for a bounded workload,
//! `OutcomeClass` tells whether all schedules agreed or diverged.

use cellgov_event::UnitId;

/// Classification of a bounded exploration run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeClass {
    /// All explored schedules produced identical committed memory.
    ScheduleStable,
    /// At least two explored schedules produced distinct committed memory.
    ScheduleSensitive,
    /// Exploration bounds were hit before enough schedules were explored
    /// to make a definitive classification.
    Inconclusive,
}

/// One explored alternate schedule and its result.
#[derive(Debug, Clone)]
pub struct ScheduleRecord {
    /// The branching-point step index where this schedule diverged.
    pub branch_step: usize,
    /// The unit forced at the branch point.
    pub alternate_choice: UnitId,
    /// Final committed-memory hash after this schedule ran.
    pub memory_hash: u64,
}

/// Result of a bounded exploration run.
#[derive(Debug, Clone)]
pub struct ExplorationResult {
    /// Final committed-memory hash from the baseline (default-schedule) run.
    pub baseline_hash: u64,
    /// Records from each alternate schedule explored.
    pub schedules: Vec<ScheduleRecord>,
    /// Classification derived from comparing all hashes.
    pub outcome: OutcomeClass,
    /// Total number of branching points in the baseline run.
    pub total_branching_points: usize,
    /// Whether the exploration stopped early because a bound was hit.
    pub bounds_hit: bool,
    /// Number of alternate schedules skipped by dependency pruning.
    pub schedules_pruned: usize,
}

#[cfg(test)]
mod tests {
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
}
