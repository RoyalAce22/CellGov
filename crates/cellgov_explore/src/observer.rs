//! Runs the baseline schedule and records every scheduling decision
//! with the full runnable set at each step.

use crate::decision::{DecisionLog, DecisionPoint};
use crate::dependency::StepFootprint;
use crate::util::StopReason;
use cellgov_core::{Runtime, RuntimeSnapshot};
use std::collections::BTreeMap;

/// Drive `rt` to stall and return the recorded [`DecisionLog`] with why
/// the run stopped.
///
/// The runtime is advanced in place; callers who need the final state
/// should inspect `rt` after the call. A reason other than
/// [`StopReason::Stalled`] means the log and `rt`'s memory hash both
/// describe a prefix of the schedule.
pub fn observe_decisions(rt: &mut Runtime) -> (DecisionLog, StopReason) {
    let (log, _, stop) = observe_decisions_with_snapshots(rt, false);
    (log, stop)
}

/// Like [`observe_decisions`], but with `capture=true` also records
/// a [`RuntimeSnapshot`] keyed by step index at every branching
/// point (>=2 runnable units). Skipping non-branching steps bounds
/// peak memory to `branching_points * sizeof(snapshot)`.
///
/// A refused commit stops the observation and is reported as
/// [`StopReason::CommitError`]: the step's effects never reached guest
/// state, so neither its [`DecisionPoint`] nor any later step describes
/// the schedule the caller asked for.
pub fn observe_decisions_with_snapshots(
    rt: &mut Runtime,
    capture: bool,
) -> (DecisionLog, BTreeMap<usize, RuntimeSnapshot>, StopReason) {
    let mut log = DecisionLog::new();
    let mut snapshots: BTreeMap<usize, RuntimeSnapshot> = BTreeMap::new();
    let stop = loop {
        let runnable: Vec<_> = rt.registry().runnable_ids().collect();
        if runnable.is_empty() {
            break StopReason::Stalled;
        }
        let step_idx = rt.steps_taken();
        if capture && runnable.len() >= 2 {
            // Snapshot must precede the step so alternates replay the
            // branching-point step itself with a different choice.
            snapshots.insert(step_idx, rt.snapshot());
        }
        match rt.step() {
            Ok(step) => {
                let mut footprint = StepFootprint::from_effects(&step.effects);
                // A write through one view of a shared mapping is a
                // write to every sibling view's bytes.
                let aliases: Vec<_> = footprint
                    .shared_writes
                    .iter()
                    .flat_map(|r| rt.shared_alias_ranges(step.unit, *r))
                    .collect();
                footprint.shared_writes.extend(aliases);
                if rt.commit_step(&step.result, &step.effects).is_err() {
                    break StopReason::CommitError;
                }
                log.push(DecisionPoint {
                    step: step_idx,
                    runnable,
                    chosen: step.unit,
                    footprint,
                });
            }
            Err(_) => break StopReason::StepError,
        }
    };
    (log, snapshots, stop)
}

#[cfg(test)]
#[path = "tests/observer_tests.rs"]
mod tests;
