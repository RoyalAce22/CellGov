//! Main bounded enumerator: baseline run then one replay per
//! non-pruned alternate branching-point choice.
//!
//! Alternates restore from a [`cellgov_core::RuntimeSnapshot`]
//! captured during the baseline rather than re-running from step 0.

use crate::classify::{BaselineRun, ExplorationResult};
use crate::config::ExplorationConfig;
use crate::observer::observe_decisions_with_snapshots;
use crate::prescribed::PrescribedScheduler;
use crate::util::{classify_iteration, for_each_alternate, run_to_stall};
use cellgov_core::Runtime;

/// Run bounded schedule exploration on a workload.
///
/// Returns `None` if the baseline run has no branching points;
/// [`explore_window`] reports the baseline's facts in that case
/// instead. `explore` calls `make_runtime` exactly once. Exploration
/// stops at `config.max_schedules` alternates and each replay at
/// `config.max_steps_per_run` steps.
///
/// A replay that stops for any reason other than a stall sets
/// [`ExplorationResult::bounds_hit`] and cannot contribute a
/// divergence. A baseline that itself stopped short withdraws every
/// divergence claim, because `baseline_hash` is then a prefix hash.
pub fn explore<F>(make_runtime: F, config: &ExplorationConfig) -> Option<ExplorationResult>
where
    F: FnMut() -> Runtime,
{
    let result = explore_window(make_runtime, config);
    (result.total_branching_points > 0).then_some(result)
}

/// [`explore`] that reports the baseline's facts even when the run
/// holds no branching point.
///
/// A driver that explores a window of a longer run reads
/// [`ExplorationResult::baseline_steps`] and
/// [`ExplorationResult::baseline_stop`] to say which part of the
/// workload its verdict covers. With no branching point the verdict
/// rests on the baseline alone:
///
/// - schedule-stable when the baseline ran itself out;
/// - inconclusive when it stopped short or committed no step.
pub fn explore_window<F>(mut make_runtime: F, config: &ExplorationConfig) -> ExplorationResult
where
    F: FnMut() -> Runtime,
{
    let mut rt_baseline = make_runtime();
    let (log, snapshots, baseline_stop) = observe_decisions_with_snapshots(&mut rt_baseline, true);
    let baseline = BaselineRun {
        hash: rt_baseline.committed_memory_hash(),
        steps: log.len(),
        stop: baseline_stop,
    };
    let baseline_hash = baseline.hash;

    let total_branching_points = log.branching_count();

    // Read before the first replay: `Runtime::restore_into` overwrites
    // the whole LV2 host from the snapshot, so the baseline's record is
    // gone the moment an alternate restores over it.
    let mut first_invariant_break = rt_baseline
        .lv2_host()
        .observability()
        .first_invariant_break_line();

    let mut iter = for_each_alternate(&log, config, baseline_hash, |step, alt| {
        let snap = snapshots
            .get(&step)
            .expect("observer must snapshot every branching point");
        rt_baseline.restore_into(snap);
        rt_baseline.set_scheduler(PrescribedScheduler::single_choice(alt));
        let stop = run_to_stall(&mut rt_baseline, config.max_steps_per_run);
        // The next replay restores over this one's record, so a break
        // only this replay found is readable only here.
        if first_invariant_break.is_none() {
            first_invariant_break = rt_baseline
                .lv2_host()
                .observability()
                .first_invariant_break_line();
        }
        (rt_baseline.committed_memory_hash(), stop)
    });

    if baseline.stop.is_truncated() {
        iter.mark_baseline_truncated();
    }

    classify_iteration(
        iter,
        baseline,
        total_branching_points,
        first_invariant_break,
    )
}

#[cfg(test)]
#[path = "tests/explorer_tests.rs"]
mod tests;
