//! Outcome classification types for an exploration run.

use crate::util::StopReason;
use cellgov_event::UnitId;

/// Verdict of a bounded exploration run.
///
/// `IntoStaticStr` derive is the single source of truth for the
/// human / JSON wire-form: `schedule-stable`, `schedule-sensitive`,
/// `inconclusive`. `report::outcome_label` delegates to the derived
/// `From<&OutcomeClass> for &'static str` impl.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::VariantArray, strum::IntoStaticStr)]
pub enum OutcomeClass {
    /// Every schedule explored produced identical committed memory.
    ///
    /// [`ExplorationResult::classes_explored`] says how far the verdict
    /// reaches. A count means the search covered one execution per
    /// equivalence class and hit no bound, so the verdict covers every
    /// schedule. An empty count holds the verdict to the schedules the
    /// search sampled.
    #[strum(serialize = "schedule-stable")]
    ScheduleStable,
    /// At least two explored schedules produced distinct committed memory.
    #[strum(serialize = "schedule-sensitive")]
    ScheduleSensitive,
    /// Bounds were hit before a divergence was observed or ruled out.
    #[strum(serialize = "inconclusive")]
    Inconclusive,
}

/// What the default-schedule run of an exploration produced.
///
/// The exploration measures every alternate against
/// [`BaselineRun::hash`]. `steps` and `stop` say which part of the
/// workload that hash covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaselineRun {
    /// Committed-memory hash after the baseline ran.
    pub hash: u64,
    /// Steps the baseline committed; see
    /// [`ExplorationResult::baseline_steps`].
    pub steps: usize,
    /// Why the baseline stopped.
    pub stop: StopReason,
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
    /// Why this alternate's own replay stopped.
    ///
    /// Distinct from [`Self::truncated`], which a prefix baseline sets
    /// on every record whatever each replay itself did.
    pub stop: StopReason,
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
    /// Steps the baseline committed, which with [`Self::baseline_stop`]
    /// says how much of the workload [`Self::baseline_hash`] covers.
    ///
    /// The count omits a step whose commit the runtime refused: its
    /// effects never reached guest state. A caller that adds the count
    /// to a starting step index therefore lands one short of
    /// [`cellgov_core::Runtime::steps_taken`].
    pub baseline_steps: usize,
    /// Why the baseline stopped.
    pub baseline_stop: StopReason,
    /// Records from each non-pruned alternate schedule explored.
    pub schedules: Vec<ScheduleRecord>,
    /// Verdict derived from comparing all hashes.
    pub outcome: OutcomeClass,
    /// Total branching points observed in the baseline run.
    pub total_branching_points: usize,
    /// Equivalence classes the search covered.
    ///
    /// Empty for a search that runs more than one execution per class,
    /// and for one that hit a bound before it covered every class. See
    /// [`OutcomeClass::ScheduleStable`] for what a count changes about
    /// a verdict.
    ///
    /// [`Self::reversals_dropped`] separates the two ways this is
    /// empty: a search that claims nothing, and one whose claim a drop
    /// withdrew.
    pub classes_explored: Option<usize>,
    /// Reversals the search owed and could not deliver.
    ///
    /// A race names a pair whose later unit cannot run where the
    /// reversal would go, so the search drops that branch. A drop gives
    /// up at least one equivalence class, which is why any drop empties
    /// [`Self::classes_explored`].
    ///
    /// A run that dropped one reversal and one that gave up half its
    /// classes both report no count, and only this separates them. It
    /// counts branches over every execution the search ran, and one
    /// branch can carry more than one owed sequence, so it is not a
    /// count of the classes given up.
    ///
    /// Zero beside an empty count means no drop withdrew the count: a
    /// bound or a short stop did, or the search claims no count of its
    /// own. [`crate::explore_backtrack`] counts no drop, so its zero is
    /// no claim.
    pub reversals_dropped: usize,
    /// True if the `max_schedules` bound was hit, or if the baseline or
    /// any replay stopped before the workload finished.
    ///
    /// [`Self::outcome`] can be [`OutcomeClass::Inconclusive`] with this
    /// false, when the baseline committed no step at all.
    pub bounds_hit: bool,
    /// Starts the search dropped before they reached a record.
    ///
    /// What the count names depends on the search:
    ///
    /// - an alternate whose two units the execution proved independent;
    /// - an execution the search dropped because every runnable unit
    ///   was already explored from its prefix.
    ///
    /// Neither costs a class, so the count measures work saved.
    pub schedules_pruned: usize,
    /// Alternates whose recorded hash covers only a prefix of their
    /// schedule, counting every record when the baseline itself
    /// stopped short.
    pub schedules_truncated: usize,
    /// Alternates whose replay stopped on a
    /// [`crate::util::StopClass::Refusal`], a subset of
    /// [`Self::schedules_truncated`].
    ///
    /// Only a refusal names a defect in the model:
    ///
    /// - a bound stops a replay the caller capped;
    /// - a blocked replay stops on the workload's own state.
    pub schedules_refused: usize,
    /// The exploration's first host invariant break as one line for the
    /// caller to report: the baseline's, or the first replay that broke
    /// one when the baseline broke none.
    ///
    /// Each run restores the LV2 host from the search's start
    /// snapshot, so the search reads the line after every run. The next
    /// restore overwrites it.
    pub first_invariant_break: Option<String>,
}

#[cfg(test)]
#[path = "tests/classify_tests.rs"]
mod tests;
