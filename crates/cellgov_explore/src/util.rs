//! Shared helpers used by the exploration entry points.

use crate::classify::{ExplorationResult, OutcomeClass, ScheduleRecord};
use crate::config::ExplorationConfig;
use crate::decision::DecisionLog;
use cellgov_core::Runtime;
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
    StepError,
    /// A commit was refused, so the step's effects never reached guest
    /// state.
    CommitError,
}

impl StopReason {
    /// True when the run stopped before the workload finished.
    pub fn is_truncated(self) -> bool {
        !matches!(self, StopReason::Stalled)
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
                if rt.commit_step(&step.result, &step.effects).is_err() {
                    return StopReason::CommitError;
                }
                steps += 1;
            }
            Err(_) => return StopReason::StepError,
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
    /// True if the `max_schedules` bound was hit.
    pub bounds_hit: bool,
    /// True if at least one alternate produced a different memory hash.
    pub found_divergence: bool,
    /// Alternates skipped by dependency pruning.
    pub schedules_pruned: usize,
}

/// Iterate each non-pruned alternate at every branching point.
///
/// `process` is called with `(branch_step, alternate_unit)` and returns
/// that alternate's final memory hash. Iteration stops early when the
/// `max_schedules` bound is reached.
pub fn for_each_alternate<F>(
    log: &DecisionLog,
    config: &ExplorationConfig,
    baseline_hash: u64,
    mut process: F,
) -> AlternateIteration
where
    F: FnMut(usize, UnitId) -> u64,
{
    let branching: Vec<_> = log.branching_points().collect();
    let mut schedules = Vec::new();
    let mut bounds_hit = false;
    let mut found_divergence = false;
    let mut schedules_pruned: usize = 0;

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

            let hash = process(bp.step, alt);
            if hash != baseline_hash {
                found_divergence = true;
            }
            schedules.push(ScheduleRecord {
                branch_step: bp.step,
                alternate_choice: alt,
                memory_hash: hash,
            });
        }
    }

    AlternateIteration {
        schedules,
        bounds_hit,
        found_divergence,
        schedules_pruned,
    }
}

/// Collapse an [`AlternateIteration`] tally into an
/// [`ExplorationResult`].
pub fn classify_iteration(
    iter: AlternateIteration,
    baseline_hash: u64,
    total_branching_points: usize,
) -> ExplorationResult {
    let outcome = if iter.found_divergence {
        OutcomeClass::ScheduleSensitive
    } else if iter.bounds_hit {
        OutcomeClass::Inconclusive
    } else {
        OutcomeClass::ScheduleStable
    };
    ExplorationResult {
        baseline_hash,
        schedules: iter.schedules,
        outcome,
        total_branching_points,
        bounds_hit: iter.bounds_hit,
        schedules_pruned: iter.schedules_pruned,
    }
}
