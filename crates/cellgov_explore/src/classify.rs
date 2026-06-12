//! Outcome classification types for an exploration run.

use cellgov_event::UnitId;

/// Verdict of a bounded exploration run.
///
/// `IntoStaticStr` derive is the single source of truth for the
/// human / JSON wire-form: `schedule-stable`, `schedule-sensitive`,
/// `inconclusive`. `report::outcome_label` delegates to the derived
/// `From<&OutcomeClass> for &'static str` impl.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::VariantArray, strum::IntoStaticStr)]
pub enum OutcomeClass {
    /// All explored schedules produced identical committed memory.
    #[strum(serialize = "schedule-stable")]
    ScheduleStable,
    /// At least two explored schedules produced distinct committed memory.
    #[strum(serialize = "schedule-sensitive")]
    ScheduleSensitive,
    /// Bounds were hit before a divergence was observed or ruled out.
    #[strum(serialize = "inconclusive")]
    Inconclusive,
}

/// One explored alternate schedule and its committed-memory hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleRecord {
    /// Step index of the branching point this alternate diverges at.
    pub branch_step: usize,
    /// Unit forced at the branch point.
    pub alternate_choice: UnitId,
    /// Final committed-memory hash after the alternate ran.
    pub memory_hash: u64,
}

/// Aggregate result of a bounded exploration run.
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[path = "tests/classify_tests.rs"]
mod tests;
