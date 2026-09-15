//! Shared helpers used by the exploration entry points.

use crate::classify::{BaselineRun, ExplorationResult, OutcomeClass, ScheduleRecord};
use cellgov_core::{CommitError, Runtime, StepError};
use cellgov_effects::FaultKind;

/// Why [`run_to_stall`] returned.
///
/// [`StopReason::Stalled`] and [`StopReason::Deadlocked`] end a
/// maximal execution. Every other reason leaves a prefix of the
/// schedule, whose memory hash is no finished run's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumCount)]
pub enum StopReason {
    /// No unit was runnable: the workload finished.
    Stalled,
    /// A unit is parked and no wake source is left to wake it.
    ///
    /// The execution is maximal, so the run answers for it. The search
    /// reads its races, and a schedule that deadlocks where another
    /// finishes is a divergence.
    Deadlocked,
    /// A child is parked behind a staged init pass no explorer runs.
    ///
    /// The pass holds every other runnable unit `Blocked` across the
    /// child's `module_start` and restores them after, through no
    /// effect, so no footprint records either half. A relation that
    /// cannot see those parks would call steps independent that a pass
    /// separated, which is the silent direction. The search stops
    /// instead of exploring a window it cannot reason about.
    ///
    /// Every step loop reads this before it starts a step, so a runtime
    /// handed over with a pass already pending runs none. The step that
    /// stages one commits and reaches no decision point, so a log this
    /// stop ends covers one step less than the run's memory hash.
    ChildInitUnserved,
    /// The caller's step cap refused a step the execution had left to
    /// take, so what the run reports covers a prefix.
    ///
    /// An execution that reaches the cap with nothing left to run
    /// reports the stop its own work justifies instead. Every step loop
    /// that reads a cap asks
    /// [`cellgov_core::Runtime::can_take_another_step`] before it
    /// answers with this.
    StepBound,
    /// `Runtime::step` refused.
    StepError(StepError),
    /// `Runtime::commit_step` refused, so the step's effects never
    /// reached guest state.
    CommitError(CommitError),
    /// A unit faulted and the commit discarded its batch.
    ///
    /// The faulted unit leaves the runnable set, so a run that went on
    /// past the fault would report a stall two steps later.
    Faulted(FaultKind),
}

/// What a [`StopReason`] says about the run that reported it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::VariantArray, strum::IntoStaticStr)]
pub enum StopClass {
    /// The workload ran itself out.
    #[strum(serialize = "finished")]
    Finished,
    /// A cap the caller set stopped a run with work still to do.
    #[strum(serialize = "bound")]
    Bound,
    /// No unit can run and nothing can wake one.
    #[strum(serialize = "blocked")]
    Blocked,
    /// The model refused the step or its commit.
    #[strum(serialize = "refusal")]
    Refusal,
    /// A unit yielded because a step could not complete.
    ///
    /// Separate from [`StopClass::Refusal`]: a refusal is the model
    /// declining to do the thing, and a fault is the guest's own step
    /// failing. Only the first names a defect in the model.
    #[strum(serialize = "fault")]
    Fault,
    /// The search will not answer for this window.
    ///
    /// Nothing is wrong with the model or the guest: the window holds
    /// something the relation cannot see, so no verdict over it would
    /// mean anything. Separate from [`StopClass::Refusal`] because a
    /// reader chasing a model defect should not be sent here, and
    /// separate from [`StopClass::Bound`] because no cap the caller can
    /// raise would help.
    #[strum(serialize = "unserved")]
    Unserved,
}

impl StopClass {
    /// The one-word label the human and JSON reports both print.
    pub fn label(self) -> &'static str {
        <&'static str>::from(&self)
    }
}

impl StopReason {
    /// True when the run stopped before its execution was maximal.
    ///
    /// A deadlock is no truncation: no schedule extends the execution
    /// past it, so its hash answers for the whole run.
    pub fn is_truncated(self) -> bool {
        !matches!(self, StopReason::Stalled | StopReason::Deadlocked)
    }

    /// Which class this stop belongs to.
    pub fn class(self) -> StopClass {
        match self {
            Self::Stalled => StopClass::Finished,
            Self::Deadlocked => StopClass::Blocked,
            // The two caps a caller holds: the exploration's own
            // per-replay cap, and the runtime's step cap.
            Self::StepBound | Self::StepError(StepError::MaxStepsExceeded) => StopClass::Bound,
            Self::StepError(StepError::NoRunnableUnit | StepError::AllBlocked) => {
                StopClass::Blocked
            }
            // Guest time reaching u64::MAX is no cap anyone can raise.
            Self::StepError(StepError::TimeOverflow | StepError::SchedulerNotReinstalled)
            | Self::CommitError(_) => StopClass::Refusal,
            Self::Faulted(_) => StopClass::Fault,
            // Neither the model nor the guest is at fault: the window
            // holds what the relation cannot see, so the search answers
            // for none of it.
            Self::ChildInitUnserved => StopClass::Unserved,
        }
    }
}

impl std::fmt::Display for StopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stalled => f.write_str("stalled"),
            Self::Deadlocked => f.write_str("deadlocked: a unit is parked with no wake source"),
            Self::ChildInitUnserved => f.write_str(
                "the window spans a spawn whose staged init pass parks every other unit \
                 through no effect, which no footprint records",
            ),
            Self::StepBound => f.write_str("replay step bound reached"),
            Self::StepError(e) => write!(f, "step refused: {e}"),
            Self::CommitError(e) => write!(f, "commit refused: {e}"),
            Self::Faulted(kind) => write!(f, "unit faulted: {kind:?}"),
        }
    }
}

/// Drive `rt` until it stops, committing each step immediately.
///
/// `Runtime::step` decides when a run is over. An empty runnable set
/// is not that decision. The runtime warps guest time to the earlier
/// of the next DMA completion and the next timer deadline. It fires
/// what is due, then schedules whatever that wakes. The step reports
/// `NoRunnableUnit` where every unit finished, and `AllBlocked` where
/// a parked unit has no wake source left. Both end the execution, and
/// neither truncates it.
///
/// A refused commit stops the run: its effects never landed, so every
/// later step would build on a state the schedule did not produce. A
/// fault stops it for the same reason -- the commit discards its
/// batch, and the faulted unit leaves the runnable set. A staged child
/// init the host has not run stops it because the relation cannot see
/// the parks it holds; see [`StopReason::ChildInitUnserved`].
pub fn run_to_stall(rt: &mut Runtime, max_steps: usize) -> StopReason {
    let mut steps = 0;
    loop {
        // Read before the cap: a window nothing can model is no
        // caller's bound. A driver can hand over a runtime that already
        // carries a pending pass, and the step below would run under
        // parks no footprint records.
        if rt.has_pending_child_init() {
            return StopReason::ChildInitUnserved;
        }
        // The cap refuses to start a step, so it answers only where
        // there was one to start. See `Runtime::can_take_another_step`.
        let at_cap = steps >= max_steps;
        if at_cap && rt.can_take_another_step() {
            return StopReason::StepBound;
        }
        match rt.step() {
            Ok(step) => {
                // The predicate is a second reading of the question
                // `Runtime::step` itself answers. A step that runs past
                // the cap is the two disagreeing, and the cap then
                // bounds nothing.
                debug_assert!(
                    !at_cap,
                    "the cap was reached, the predicate saw no step left, and one ran",
                );
                // The commit discards the batch and counts it, so the
                // fault read comes after it. A refusal outranks a
                // fault, as the boot's own step loop ranks them.
                if let Err(e) = rt.commit_step(&step.result, &step.effects) {
                    return StopReason::CommitError(e);
                }
                // The pass this parks behind reaches no footprint, so
                // the relation cannot answer for the steps after it.
                // Ahead of the fault for the same reason the commit
                // refusal above is: a refusal outranks a fault.
                if rt.has_pending_child_init() {
                    return StopReason::ChildInitUnserved;
                }
                if let Some(kind) = step.result.fault {
                    return StopReason::Faulted(kind);
                }
                steps += 1;
            }
            Err(StepError::NoRunnableUnit) => return StopReason::Stalled,
            Err(StepError::AllBlocked) => return StopReason::Deadlocked,
            Err(e) => return StopReason::StepError(e),
        }
    }
}

/// Tally one search kept over the executions it ran.
///
/// Every search collapses its own tally through
/// [`classify_iteration`], which holds the truncation rules.
pub struct AlternateIteration {
    /// Per-schedule outcomes.
    pub schedules: Vec<ScheduleRecord>,
    /// True if the `max_schedules` bound was hit, or if any replay
    /// returned a truncating [`StopReason`].
    pub bounds_hit: bool,
    /// True if at least one untruncated alternate produced a different
    /// memory hash.
    ///
    /// A truncated replay never sets this: its hash is a prefix of the
    /// alternate schedule, and a prefix differs from a completed
    /// baseline whether or not the workload is schedule-sensitive.
    pub found_divergence: bool,
    /// Starts the search dropped before they reached a record; see
    /// [`ExplorationResult::schedules_pruned`].
    pub schedules_pruned: usize,
    /// Alternates whose replay stopped on a truncating [`StopReason`].
    pub schedules_truncated: usize,
    /// Alternates whose replay stopped on a [`StopClass::Refusal`].
    pub schedules_refused: usize,
}

impl AlternateIteration {
    /// Withdraw every divergence claim because the baseline itself
    /// stopped short.
    ///
    /// Alternates are compared against `baseline_hash`; when that hash
    /// came from a prefix of the baseline schedule, neither a match nor
    /// a mismatch says anything about schedule sensitivity, so the pass
    /// can only report inconclusive.
    pub fn mark_baseline_truncated(&mut self) {
        self.found_divergence = false;
        self.bounds_hit = true;
        self.schedules_truncated = self.schedules.len();
        for record in &mut self.schedules {
            record.truncated = true;
        }
    }
}

/// Collapse an [`AlternateIteration`] tally into an
/// [`ExplorationResult`].
///
/// A baseline that committed no step measured nothing, so it cannot
/// support [`OutcomeClass::ScheduleStable`] however empty the tally is.
pub fn classify_iteration(
    iter: AlternateIteration,
    baseline: BaselineRun,
    total_branching_points: usize,
    first_invariant_break: Option<String>,
) -> ExplorationResult {
    let outcome = if iter.found_divergence {
        OutcomeClass::ScheduleSensitive
    } else if iter.bounds_hit || baseline.steps == 0 {
        OutcomeClass::Inconclusive
    } else {
        OutcomeClass::ScheduleStable
    };
    ExplorationResult {
        baseline_hash: baseline.hash,
        baseline_steps: baseline.steps,
        baseline_stop: baseline.stop,
        schedules: iter.schedules,
        outcome,
        total_branching_points,
        // `explore_optimal` overwrites both of these on the result this
        // returns, and `explore_backtrack` the drop count. The bounded
        // enumerator overwrites neither, so for it the zeros below are
        // silence rather than a measurement.
        classes_explored: None,
        reversals_dropped: 0,
        bounds_hit: iter.bounds_hit,
        schedules_pruned: iter.schedules_pruned,
        schedules_truncated: iter.schedules_truncated,
        schedules_refused: iter.schedules_refused,
        first_invariant_break,
    }
}

#[cfg(test)]
#[path = "tests/util_tests.rs"]
mod tests;
