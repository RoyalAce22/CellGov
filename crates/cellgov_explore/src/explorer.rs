//! The crate's exploration entry points, over optimal DPOR:
//! [`crate::optimal::explore_optimal`] does the search, and these two
//! wrap it in the shape a driver asks for -- one reports nothing when a
//! workload holds no choice, the other reports the baseline's facts
//! either way.

use crate::classify::ExplorationResult;
use crate::config::ExplorationConfig;
use crate::optimal::explore_optimal;
use cellgov_core::Runtime;

/// Run bounded schedule exploration on a workload.
///
/// Returns `None` if the baseline run has no branching points;
/// [`explore_window`] reports the baseline's facts in that case
/// instead.
///
/// The search calls `make_runtime` once, snapshots the runtime it
/// returns, and restores that snapshot per execution. A driver that
/// hands over a runtime it already advanced -- a window of a title
/// boot -- can explore from there, handing it over as `move || rt`.
///
/// An execution that stopped short contributes no divergence; see
/// [`crate::classify::ScheduleRecord::truncated`].
pub fn explore<F>(make_runtime: F, config: &ExplorationConfig) -> Option<ExplorationResult>
where
    F: FnOnce() -> Runtime,
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
pub fn explore_window<F>(make_runtime: F, config: &ExplorationConfig) -> ExplorationResult
where
    F: FnOnce() -> Runtime,
{
    explore_optimal(make_runtime, config)
}

#[cfg(test)]
#[path = "tests/explorer_tests.rs"]
mod tests;
