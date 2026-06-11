//! `bench-boot` step driver: throughput-only loop with the shared
//! [`super::verdict::classify_step_outcome`] precedence rules. The
//! `run-game` driver with full diagnostics lives in [`super::driver`].

use cellgov_core::{Runtime, StepError};

use crate::game::manifest;
use crate::game::step_loop::verdict::{classify_step_outcome, StepVerdict};

/// `CommitFault` and `StepFault` both surface as `BootOutcome::Fault`;
/// `NoRunnableUnit` and `AllBlocked` both surface as `ProcessExit`.
pub(in crate::game) fn bench_step_loop(
    rt: &mut Runtime,
    checkpoint: manifest::CheckpointTrigger,
    steps: &mut usize,
) -> cellgov_compare::BootOutcome {
    use cellgov_compare::BootOutcome;
    use manifest::CheckpointTrigger;
    let target_pc = match checkpoint {
        CheckpointTrigger::Pc(addr) => Some(addr),
        _ => None,
    };
    loop {
        match rt.step() {
            Ok(step) => {
                *steps += 1;
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
                // bench-boot does not call Runtime::restore_into.
                unreachable!(
                    "bench-boot does not call Runtime::restore_into; \
                     reaching this arm means a new caller added a \
                     restore path without rethinking the dispatch."
                );
            }
        }
    }
}
