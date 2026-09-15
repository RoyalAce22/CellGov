//! `boot bench` step driver: throughput-only loop with the shared
//! [`super::verdict::classify_step_outcome`] precedence rules. The
//! `boot run` driver with full diagnostics lives in [`super::driver`].

use std::rc::Rc;

use cellgov_core::{Runtime, StepError};
use cellgov_terminal::progress::ProgressSink;

use crate::manifest;
use crate::step_loop::verdict::{classify_step_outcome, StepVerdict};
use crate::step_loop::STEP_REPORT_BATCH;
use crate::{BootError, BootSink, ChildInitPlans};

/// Drive the runtime to `checkpoint` with no per-step diagnostics.
///
/// `CommitFault` and `StepFault` both surface as `BootOutcome::Fault`;
/// `NoRunnableUnit` and `AllBlocked` both surface as `ProcessExit`.
///
/// # Errors
///
/// A spawned child's init pass that could not finish.
pub fn bench_step_loop(
    rt: &mut Runtime,
    checkpoint: manifest::CheckpointTrigger,
    steps: &mut usize,
    child_init: &ChildInitPlans,
    progress: &dyn ProgressSink,
    sink: &Rc<dyn BootSink>,
) -> Result<cellgov_compare::BootOutcome, BootError> {
    // The tail report takes `steps` modulo the batch. That remainder
    // is the partial batch only when the count starts at zero.
    debug_assert_eq!(*steps, 0, "the step counter enters the loop at zero");
    let outcome = drive(rt, checkpoint, steps, child_init, progress, sink);
    // The loop reports whole batches, so the bar needs the partial
    // batch it ended on to reach the step count the run reports.
    progress.advanced((*steps % STEP_REPORT_BATCH) as u64);
    outcome
}

/// Split from the wrapper so every exit passes the tail report.
fn drive(
    rt: &mut Runtime,
    checkpoint: manifest::CheckpointTrigger,
    steps: &mut usize,
    child_init: &ChildInitPlans,
    progress: &dyn ProgressSink,
    sink: &Rc<dyn BootSink>,
) -> Result<cellgov_compare::BootOutcome, BootError> {
    use cellgov_compare::BootOutcome;
    use manifest::CheckpointTrigger;
    let target_pc = match checkpoint {
        CheckpointTrigger::Pc(addr) => Some(addr),
        _ => None,
    };
    loop {
        // Same placement as `super::driver`; see the comment there.
        // The pass retires `rt.step()` calls that `steps` never counts.
        // The bar counts what `steps` counts: an anchor records its
        // finish line from `steps`.
        if rt.has_pending_child_init() {
            crate::child_init::run_pending_child_inits(rt, child_init, sink)?;
        }
        match rt.step() {
            Ok(step) => {
                *steps += 1;
                if (*steps).is_multiple_of(STEP_REPORT_BATCH) {
                    progress.advanced(STEP_REPORT_BATCH as u64);
                }
                let commit_result = rt.commit_step(&step.result, &step.effects);
                match classify_step_outcome(&step.result, &commit_result, checkpoint, target_pc) {
                    StepVerdict::Continue => {}
                    StepVerdict::RsxCheckpoint(_) => return Ok(BootOutcome::RsxWriteCheckpoint),
                    StepVerdict::CommitFault | StepVerdict::StepFault => {
                        return Ok(BootOutcome::Fault)
                    }
                    StepVerdict::PcReached(addr) => return Ok(BootOutcome::PcReached(addr)),
                }
            }
            Err(StepError::NoRunnableUnit) | Err(StepError::AllBlocked) => {
                return Ok(BootOutcome::ProcessExit);
            }
            Err(StepError::MaxStepsExceeded) => return Ok(BootOutcome::MaxSteps),
            Err(StepError::TimeOverflow) => return Ok(BootOutcome::TimeOverflow),
            Err(StepError::SchedulerNotReinstalled) => {
                // boot bench does not call Runtime::restore_into.
                unreachable!(
                    "boot bench does not call Runtime::restore_into; \
                     reaching this arm means a new caller added a \
                     restore path without rethinking the dispatch."
                );
            }
        }
    }
}
