//! Runs one schedule and records every scheduling decision with the
//! full runnable set at each step.

use crate::decision::{DecisionLog, DecisionPoint};
use crate::dependency::StepFootprint;
use crate::util::StopReason;
use cellgov_core::Runtime;

/// Drive `rt` until it stops and return the recorded [`DecisionLog`]
/// with why the run stopped.
///
/// A truncating stop ([`StopReason::is_truncated`]) means the log and
/// `rt`'s memory hash both describe a prefix of the schedule. The two
/// cover the same steps except under
/// [`StopReason::ChildInitUnserved`], where the hash covers one step
/// the log leaves out.
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
        // Read before the cap: a runtime handed over with a pending pass
        // would run under parks no footprint records. The same loop
        // shape as `crate::util::run_to_stall`.
        if rt.has_pending_child_init() {
            break StopReason::ChildInitUnserved;
        }
        // The cap refuses to start a step, so it answers only where
        // there was one to start. See `Runtime::can_take_another_step`.
        let at_cap = max_steps.is_some_and(|cap| committed >= cap);
        if at_cap && rt.can_take_another_step() {
            break StopReason::StepBound;
        }
        let step_idx = rt.steps_taken();
        match rt.step() {
            Ok(step) => {
                // The predicate re-reads the question `Runtime::step`
                // answers; a step past the cap means the two disagree,
                // and the cap then bounds nothing.
                debug_assert!(
                    !at_cap,
                    "the cap was reached, the predicate saw no step left, and one ran",
                );
                // A warp inside the step can widen the set the
                // scheduler chose from, so the point reads it here.
                let runnable: Vec<_> = rt.last_runnable().to_vec();
                let mut footprint =
                    StepFootprint::from_step(step.unit, step.result.yield_reason, &step.effects);
                if let Err(e) = rt.commit_step(&step.result, &step.effects) {
                    break StopReason::CommitError(e);
                }
                footprint.note_commit(rt, step.unit);
                // The pass this parks behind reaches no footprint, so
                // the relation cannot answer for the steps after it.
                if rt.has_pending_child_init() {
                    break StopReason::ChildInitUnserved;
                }
                // A discarded batch reached no guest state, so the step
                // gets no point, as a refused commit gets none.
                if let Some(kind) = step.result.fault {
                    break StopReason::Faulted(kind);
                }
                log.push(DecisionPoint {
                    step: step_idx,
                    runnable,
                    chosen: step.unit,
                    footprint,
                });
                committed += 1;
            }
            Err(cellgov_core::StepError::NoRunnableUnit) => break StopReason::Stalled,
            Err(cellgov_core::StepError::AllBlocked) => break StopReason::Deadlocked,
            Err(e) => break StopReason::StepError(e),
        }
    };
    (log, stop)
}

#[cfg(test)]
#[path = "tests/observer_tests.rs"]
mod tests;
