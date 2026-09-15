//! Shared helpers used by the exploration entry points.

use crate::classify::{BaselineRun, ExplorationResult, OutcomeClass, ScheduleRecord};
use crate::config::ExplorationConfig;
use crate::decision::DecisionLog;
use cellgov_core::{CommitError, Runtime, StepError};
use cellgov_event::UnitId;

/// Why [`run_to_stall`] returned.
///
/// Only [`StopReason::Stalled`] means the workload ran itself out. Every
/// other reason leaves a prefix of the schedule, whose memory hash must
/// not be read as a finished run's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// No unit was runnable: the workload finished.
    Stalled,
    /// `max_steps` was reached with work still runnable.
    StepBound,
    /// `Runtime::step` refused.
    StepError(StepError),
    /// `Runtime::commit_step` refused, so the step's effects never
    /// reached guest state.
    CommitError(CommitError),
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
    /// No unit can run and nothing can wake one: a parked unit with no
    /// wake source, or a registry whose units all faulted or finished.
    #[strum(serialize = "blocked")]
    Blocked,
    /// The model refused the step or its commit.
    #[strum(serialize = "refusal")]
    Refusal,
}

impl StopClass {
    /// The one-word label the human and JSON reports both print.
    pub fn label(self) -> &'static str {
        <&'static str>::from(&self)
    }
}

impl StopReason {
    /// True when the run stopped before the workload finished.
    pub fn is_truncated(self) -> bool {
        !matches!(self, StopReason::Stalled)
    }

    /// Which class this stop belongs to.
    pub fn class(self) -> StopClass {
        match self {
            Self::Stalled => StopClass::Finished,
            // The two caps a caller holds: the exploration's own
            // per-replay cap, and the runtime's step cap.
            Self::StepBound | Self::StepError(StepError::MaxStepsExceeded) => StopClass::Bound,
            Self::StepError(StepError::NoRunnableUnit | StepError::AllBlocked) => {
                StopClass::Blocked
            }
            // Guest time reaching u64::MAX is no cap anyone can raise.
            Self::StepError(StepError::TimeOverflow | StepError::SchedulerNotReinstalled)
            | Self::CommitError(_) => StopClass::Refusal,
        }
    }
}

impl std::fmt::Display for StopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stalled => f.write_str("stalled"),
            Self::StepBound => f.write_str("replay step bound reached"),
            Self::StepError(e) => write!(f, "step refused: {e}"),
            Self::CommitError(e) => write!(f, "commit refused: {e}"),
        }
    }
}

/// Drive `rt` until no unit is runnable or `max_steps` is reached,
/// committing each step immediately.
///
/// A refused commit stops the run rather than continuing: its effects
/// never landed, so every later step would build on a state the
/// schedule did not produce.
pub fn run_to_stall(rt: &mut Runtime, max_steps: usize) -> StopReason {
    let mut steps = 0;
    loop {
        if rt.registry().runnable_ids().next().is_none() {
            return StopReason::Stalled;
        }
        if steps >= max_steps {
            return StopReason::StepBound;
        }
        match rt.step() {
            Ok(step) => {
                if let Err(e) = rt.commit_step(&step.result, &step.effects) {
                    return StopReason::CommitError(e);
                }
                steps += 1;
            }
            Err(e) => return StopReason::StepError(e),
        }
    }
}

/// Build an override list that defers steps `0..branch_step` to the
/// fallback and forces `choice` at `branch_step`.
pub fn build_overrides(branch_step: usize, choice: UnitId) -> Vec<Option<UnitId>> {
    let mut v = vec![None; branch_step];
    v.push(Some(choice));
    v
}

/// Tally of a pass over branching-point alternates.
pub struct AlternateIteration {
    /// Per-schedule outcomes.
    pub schedules: Vec<ScheduleRecord>,
    /// True if the `max_schedules` bound was hit, or if any replay
    /// returned a [`StopReason`] other than [`StopReason::Stalled`].
    pub bounds_hit: bool,
    /// True if at least one alternate that ran to a stall produced a
    /// different memory hash.
    ///
    /// A truncated replay never sets this: its hash is a prefix of the
    /// alternate schedule, and a prefix differs from a completed
    /// baseline whether or not the workload is schedule-sensitive.
    pub found_divergence: bool,
    /// Alternates skipped by dependency pruning.
    pub schedules_pruned: usize,
    /// Alternates whose replay stopped before the workload finished.
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

/// Iterate each non-pruned alternate at every branching point.
///
/// `process` is called with `(branch_step, alternate_unit)` and returns
/// that alternate's final memory hash together with why its replay
/// stopped. Iteration stops early when the `max_schedules` bound is
/// reached.
///
/// Only a replay that reports [`StopReason::Stalled`] can contribute
/// `found_divergence`; any other reason marks the pass inconclusive
/// instead.
pub fn for_each_alternate<F>(
    log: &DecisionLog,
    config: &ExplorationConfig,
    baseline_hash: u64,
    mut process: F,
) -> AlternateIteration
where
    F: FnMut(usize, UnitId) -> (u64, StopReason),
{
    let branching: Vec<_> = log.branching_points().collect();
    let mut schedules = Vec::new();
    let mut bounds_hit = false;
    let mut found_divergence = false;
    let mut schedules_pruned: usize = 0;
    let mut schedules_truncated: usize = 0;
    let mut schedules_refused: usize = 0;

    'outer: for bp in &branching {
        let default_choice = bp.chosen;
        for &alt in &bp.runnable {
            if alt == default_choice {
                continue;
            }
            if schedules.len() >= config.max_schedules {
                bounds_hit = true;
                break 'outer;
            }

            if let Some(alt_agg) = log.aggregate_footprint(alt) {
                if let Some(def_agg) = log.aggregate_footprint(default_choice) {
                    if !def_agg.conflicts(&alt_agg) {
                        schedules_pruned += 1;
                        continue;
                    }
                }
            }

            let (hash, stop) = process(bp.step, alt);
            let truncated = stop.is_truncated();
            if truncated {
                schedules_truncated += 1;
                bounds_hit = true;
                if stop.class() == StopClass::Refusal {
                    schedules_refused += 1;
                }
            } else if hash != baseline_hash {
                found_divergence = true;
            }
            schedules.push(ScheduleRecord {
                branch_step: bp.step,
                alternate_choice: alt,
                memory_hash: hash,
                stop,
                truncated,
            });
        }
    }

    AlternateIteration {
        schedules,
        bounds_hit,
        found_divergence,
        schedules_pruned,
        schedules_truncated,
        schedules_refused,
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
