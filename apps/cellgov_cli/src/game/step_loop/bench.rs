//! `boot bench` step driver: throughput-only loop with the shared
//! [`super::verdict::classify_step_outcome`] precedence rules. The
//! `boot run` driver with full diagnostics lives in [`super::driver`].

use cellgov_core::{Runtime, StepError};

use crate::game::manifest;
use crate::game::step_loop::verdict::{classify_step_outcome, StepVerdict};
use crate::progress::{ProgressSink, STEP_REPORT_BATCH};

/// `CommitFault` and `StepFault` both surface as `BootOutcome::Fault`;
/// `NoRunnableUnit` and `AllBlocked` both surface as `ProcessExit`.
pub(in crate::game) fn bench_step_loop(
    rt: &mut Runtime,
    checkpoint: manifest::CheckpointTrigger,
    steps: &mut usize,
    child_init: &crate::game::child_init::ChildInitPlans,
    progress: &dyn ProgressSink,
) -> cellgov_compare::BootOutcome {
    // The tail report takes `steps` modulo the batch. That remainder
    // is the partial batch only when the count starts at zero.
    debug_assert_eq!(*steps, 0, "the step counter enters the loop at zero");
    let outcome = drive(rt, checkpoint, steps, child_init, progress);
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
    child_init: &crate::game::child_init::ChildInitPlans,
    progress: &dyn ProgressSink,
) -> cellgov_compare::BootOutcome {
    use cellgov_compare::BootOutcome;
    use manifest::CheckpointTrigger;
    let target_pc = match checkpoint {
        CheckpointTrigger::Pc(addr) => Some(addr),
        _ => None,
    };
    loop {
        // Same placement as `super::driver`; see the comment there.
        if rt.has_pending_child_init() {
            let before = rt.steps_taken();
            crate::game::child_init::run_pending_child_inits(rt, child_init);
            // The pass retires `rt.step()` calls that `steps` never
            // counts. The denominator counts them, so an unreported
            // pass leaves the bar short.
            progress.advanced(rt.steps_taken().saturating_sub(before) as u64);
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
                    StepVerdict::RsxCheckpoint(_) => return BootOutcome::RsxWriteCheckpoint,
                    StepVerdict::CommitFault | StepVerdict::StepFault => return BootOutcome::Fault,
                    StepVerdict::PcReached(addr) => return BootOutcome::PcReached(addr),
                }
            }
            Err(StepError::NoRunnableUnit) | Err(StepError::AllBlocked) => {
                return BootOutcome::ProcessExit;
            }
            Err(StepError::MaxStepsExceeded) => return BootOutcome::MaxSteps,
            Err(StepError::TimeOverflow) => return BootOutcome::TimeOverflow,
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
