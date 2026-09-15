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
    /// True when this alternate's replay, or the baseline it is
    /// measured against, stopped before the workload finished.
    ///
    /// `memory_hash` is then a prefix hash and comparing it to
    /// `ExplorationResult::baseline_hash` says nothing about schedule
    /// sensitivity.
    pub truncated: bool,
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
    /// Alternates whose recorded hash covers only a prefix of their
    /// schedule, counting every record when the baseline itself
    /// stopped short.
    pub schedules_truncated: usize,
    /// The exploration's first host invariant break as one line for the
    /// caller to report: the baseline's, or the first replay that broke
    /// one when the baseline broke none.
    ///
    /// Each replay restores the LV2 host from a baseline snapshot, so
    /// the line is read per run rather than once at the end; the
    /// exploration consumes its runtime, and no caller can read it
    /// afterwards.
    pub first_invariant_break: Option<String>,
}

#[cfg(test)]
#[path = "tests/classify_tests.rs"]
mod tests;
