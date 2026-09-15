//! Runs one schedule and records every scheduling decision with the
//! full runnable set at each step.
//!
//! A refused commit stops the observation and reports
//! [`StopReason::CommitError`]: the step's effects never reached guest
//! state, so neither its [`DecisionPoint`] nor any later step describes
//! the schedule the caller asked for.

use crate::decision::{DecisionLog, DecisionPoint};
use crate::dependency::StepFootprint;
use crate::util::StopReason;
use cellgov_core::Runtime;

/// Drive `rt` to stall and return the recorded [`DecisionLog`] with why
/// the run stopped.
///
/// The runtime is advanced in place; callers who need the final state
/// should inspect `rt` after the call. A reason other than
/// [`StopReason::Stalled`] means the log and `rt`'s memory hash both
/// describe a prefix of the schedule.
pub fn observe_decisions(rt: &mut Runtime) -> (DecisionLog, StopReason) {
    observe(rt, None)
}

/// [`observe_decisions`] that stops after `max_steps` committed steps
/// and reports [`StopReason::StepBound`].
pub fn observe_decisions_bounded(rt: &mut Runtime, max_steps: usize) -> (DecisionLog, StopReason) {
    observe(rt, Some(max_steps))
}

fn observe(rt: &mut Runtime, max_steps: Option<usize>) -> (DecisionLog, StopReason) {
    let mut log = DecisionLog::new();
    let mut committed = 0usize;
    let stop = loop {
        let runnable: Vec<_> = rt.registry().runnable_ids().collect();
        if runnable.is_empty() {
            break StopReason::Stalled;
        }
        if max_steps.is_some_and(|cap| committed >= cap) {
            break StopReason::StepBound;
        }
        let step_idx = rt.steps_taken();
        match rt.step() {
            Ok(step) => {
                let mut footprint = StepFootprint::from_effects(&step.effects);
                // An access through one view of a shared mapping
                // reaches every sibling view's bytes, whether the
                // access writes them or reads them.
                let write_aliases: Vec<_> = footprint
                    .shared_writes
                    .iter()
                    .flat_map(|r| rt.shared_alias_ranges(step.unit, *r))
                    .collect();
                footprint.shared_writes.extend(write_aliases);
                let read_aliases: Vec<_> = footprint
                    .shared_reads
                    .iter()
                    .flat_map(|r| rt.shared_alias_ranges(step.unit, *r))
                    .collect();
                footprint.shared_reads.extend(read_aliases);
                if let Err(e) = rt.commit_step(&step.result, &step.effects) {
                    break StopReason::CommitError(e);
                }
                log.push(DecisionPoint {
                    step: step_idx,
                    runnable,
                    chosen: step.unit,
                    footprint,
                });
                committed += 1;
            }
            Err(e) => break StopReason::StepError(e),
        }
    };
    (log, stop)
}

#[cfg(test)]
#[path = "tests/observer_tests.rs"]
mod tests;
