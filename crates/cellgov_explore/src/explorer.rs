//! Main bounded enumerator: baseline run then one replay per
//! non-pruned alternate branching-point choice.
//!
//! Alternates restore from a [`cellgov_core::RuntimeSnapshot`]
//! captured during the baseline rather than re-running from step 0.

use crate::classify::ExplorationResult;
use crate::config::ExplorationConfig;
use crate::observer::observe_decisions_with_snapshots;
use crate::prescribed::PrescribedScheduler;
use crate::util::{classify_iteration, for_each_alternate, run_to_stall};
use cellgov_core::Runtime;

/// Run bounded schedule exploration on a workload.
///
/// Returns `None` if the baseline run has no branching points.
/// `make_runtime` is invoked exactly once. Exploration stops at
/// `config.max_schedules` alternates and each replay at
/// `config.max_steps_per_run` steps; a replay stopped by that
/// per-replay bound sets [`ExplorationResult::bounds_hit`].
pub fn explore<F>(mut make_runtime: F, config: &ExplorationConfig) -> Option<ExplorationResult>
where
    F: FnMut() -> Runtime,
{
    let mut rt_baseline = make_runtime();
    let (log, snapshots) = observe_decisions_with_snapshots(&mut rt_baseline, true);
    let baseline_hash = rt_baseline.committed_memory_hash();

    let total_branching_points = log.branching_count();
    if total_branching_points == 0 {
        return None;
    }

    // A replay that comes back with a unit still runnable never
    // reached a stall: `max_steps_per_run`, the runtime's own
    // `max_steps` cap, or a step error cut it short. Its hash is a
    // prefix of the alternate schedule, not a finished run, so it
    // cannot stand as evidence that the workload is schedule-stable.
    let mut replay_cut_short = false;
    let mut iter = for_each_alternate(&log, config, baseline_hash, |step, alt| {
        let snap = snapshots
            .get(&step)
            .expect("observer must snapshot every branching point");
        rt_baseline.restore_into(snap);
        rt_baseline.set_scheduler(PrescribedScheduler::single_choice(alt));
        replay_cut_short |= run_to_stall(&mut rt_baseline, config.max_steps_per_run).is_truncated();
        rt_baseline.committed_memory_hash()
    });
    // `ExplorationConfig`: exceeding either bound forces Inconclusive
    // when no divergence has been observed.
    iter.bounds_hit |= replay_cut_short;

    Some(classify_iteration(
        iter,
        baseline_hash,
        total_branching_points,
    ))
}

#[cfg(test)]
#[path = "tests/explorer_tests.rs"]
mod tests;
