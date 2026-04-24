//! Outcome classification types for an exploration run.

use cellgov_event::UnitId;

/// Verdict of a bounded exploration run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeClass {
    /// All explored schedules produced identical committed memory.
    ScheduleStable,
    /// At least two explored schedules produced distinct committed memory.
    ScheduleSensitive,
    /// Bounds were hit before a divergence was observed or ruled out.
    Inconclusive,
}

/// One explored alternate schedule and its committed-memory hash.
#[derive(Debug, Clone)]
pub struct ScheduleRecord {
    /// Step index of the branching point this alternate diverges at.
    pub branch_step: usize,
    /// Unit forced at the branch point.
    pub alternate_choice: UnitId,
    /// Final committed-memory hash after the alternate ran.
    pub memory_hash: u64,
}

/// Aggregate result of a bounded exploration run.
#[derive(Debug, Clone)]
pub struct ExplorationResult {
    /// Committed-memory hash from the default-schedule baseline run.
    pub baseline_hash: u64,
    /// Records from each non-pruned alternate schedule explored.
    pub schedules: Vec<ScheduleRecord>,
    /// Verdict derived from comparing all hashes.
    pub outcome: OutcomeClass,
    /// Total branching points observed in the baseline run.
    pub total_branching_points: usize,
    /// True if exploration stopped because a bound was hit.
    pub bounds_hit: bool,
    /// Alternates skipped by dependency pruning.
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
