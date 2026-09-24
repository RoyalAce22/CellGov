//! Outcome classification types for an exploration run.

use crate::util::StopReason;
use cellgov_event::UnitId;

/// What every verdict compares; [`cellgov_core::Runtime::observable_hash`]
/// folds it.
///
/// Two things lie outside it, so a divergence confined to either
/// reports as stable:
///
/// - the sync state [`cellgov_core::Runtime::sync_state_hash`] folds:
///   mailboxes, signal registers, reservations, mapping metadata;
/// - each unit's registers, program counter and channels.
///
/// The end of the run observes every byte, so two writes to
/// overlapping bytes are dependent whatever reads fall between them.
/// Named regions ([`crate::explore_with_regions`]) are a second
/// comparison, against an oracle, and never narrow this.
pub const OBSERVABLE: &str =
    "committed memory of every address space and every SPU's local store at the end of the run";

/// The wire form of [`OBSERVABLE`] in the JSON report.
pub const OBSERVABLE_LABEL: &str = "committed-memory-and-local-store";

/// Verdict of a bounded exploration run, with respect to
/// [`OBSERVABLE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::VariantArray, strum::IntoStaticStr)]
pub enum OutcomeClass {
    /// Every schedule explored produced an identical observable.
    ///
    /// [`ExplorationResult::classes_explored`] says how far the verdict
    /// reaches. A count means the search covered one execution per
    /// equivalence class and hit no bound, so the verdict covers every
    /// schedule. An empty count holds it to the schedules the search
    /// sampled.
    #[strum(serialize = "schedule-stable")]
    ScheduleStable,
    /// At least two explored schedules produced a distinct observable.
    #[strum(serialize = "schedule-sensitive")]
    ScheduleSensitive,
    /// A bound stopped the search before it found or ruled out a
    /// divergence.
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
    /// Observable hash ([`OBSERVABLE`]) after the baseline ran.
    pub hash: u64,
    /// Steps the baseline committed; see
    /// [`ExplorationResult::baseline_steps`].
    pub steps: usize,
    /// Why the baseline stopped.
    pub stop: StopReason,
}

/// One explored alternate schedule and its observable hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleRecord {
    /// Step index of the branching point this alternate diverges at.
    pub branch_step: usize,
    /// Unit forced at the branch point.
    pub alternate_choice: UnitId,
    /// Final observable hash ([`OBSERVABLE`]) after the alternate ran.
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
    /// Observable hash ([`OBSERVABLE`]) from the default-schedule
    /// baseline run.
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
    /// Equivalence classes the search covered; `None` when it claims
    /// none.
    ///
    /// A search claims no count when:
    ///
    /// - it runs more than one execution per class;
    /// - a replay stopped short (a bound, a fault or a refusal) before
    ///   it covered every class;
    /// - it dropped a reversal ([`Self::reversals_dropped`]).
    ///
    /// [`OutcomeClass::ScheduleStable`] says what a count changes about
    /// the verdict.
    pub classes_explored: Option<usize>,
    /// Reversals the search owed and could not deliver.
    ///
    /// A dropped reversal is one owed execution the search never ran,
    /// so it gives up at least one equivalence class and empties
    /// [`Self::classes_explored`]. The count omits what that execution's
    /// own races would owe, so no number of classes follows from it.
    ///
    /// [`crate::optimal::explore_optimal`] counts a lost sequence per
    /// frame and [`crate::backtrack::explore_backtrack`] a race per
    /// prefix; the two count different objects, so a ratio between them
    /// reads nothing.
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
    /// Only a refusal names a defect in the model;
    /// [`crate::util::StopClass`] says what each other stop names.
    pub schedules_refused: usize,
    /// The exploration's first host invariant break as one line for the
    /// caller to report: the baseline's, or the first replay that broke
    /// one when the baseline broke none.
    ///
    /// A replay's restore overwrites the LV2 host's own line, so this
    /// field is where it survives the run.
    pub first_invariant_break: Option<String>,
}

impl ExplorationResult {
    /// Every stop the search recorded: the baseline's, then each
    /// alternate's in [`Self::schedules`] order.
    pub fn stops(&self) -> impl Iterator<Item = StopReason> + '_ {
        std::iter::once(self.baseline_stop).chain(self.schedules.iter().map(|s| s.stop))
    }
}

#[cfg(test)]
#[path = "tests/classify_tests.rs"]
mod tests;
