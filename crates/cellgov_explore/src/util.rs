//! Shared utilities for the exploration engine.

use crate::classify::{ExplorationResult, OutcomeClass, ScheduleRecord};
use crate::config::ExplorationConfig;
use crate::decision::DecisionLog;
use cellgov_core::Runtime;
use cellgov_event::UnitId;

/// Run a runtime to stall or max-steps, calling step + commit_step
/// in lockstep. Returns the number of steps taken.
pub fn run_to_stall(rt: &mut Runtime, max_steps: usize) {
    let mut steps = 0;
    loop {
        if rt.registry().runnable_ids().next().is_none() {
            break;
        }
        if steps >= max_steps {
            break;
        }
        match rt.step() {
            Ok(step) => {
                let _ = rt.commit_step(&step.result);
                steps += 1;
            }
            Err(_) => break,
        }
    }
}

/// Build a PrescribedScheduler override list: `None` for all steps
/// before `branch_step`, then `Some(choice)` at the branch step.
pub fn build_overrides(branch_step: usize, choice: UnitId) -> Vec<Option<UnitId>> {
    let mut v = vec![None; branch_step];
    v.push(Some(choice));
    v
}

/// Result of iterating over branching-point alternates.
pub struct AlternateIteration {
    pub schedules: Vec<ScheduleRecord>,
    pub bounds_hit: bool,
    pub found_divergence: bool,
    pub schedules_pruned: usize,
}

/// Iterate over branching points in a decision log, calling
/// `process` for each non-pruned alternate. `process` receives
/// (branch_step, alternate_unit_id) and returns the memory hash
/// for that alternate schedule.
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

            // Dependency pruning via aggregate footprints.
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

/// Classify an `AlternateIteration` into an `ExplorationResult`.
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
