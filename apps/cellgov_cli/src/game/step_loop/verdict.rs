use cellgov_core::{CommitError, CommitOutcome};

use crate::game::manifest;

pub(in crate::game) fn rsx_write_checkpoint_addr(
    trigger: manifest::CheckpointTrigger,
    commit_result: &Result<CommitOutcome, CommitError>,
) -> Option<u64> {
    if trigger != manifest::CheckpointTrigger::FirstRsxWrite {
        return None;
    }
    if let Err(cellgov_core::CommitError::Memory(cellgov_mem::MemError::ReservedWrite {
        addr,
        region: "rsx",
    })) = commit_result
    {
        Some(*addr)
    } else {
        None
    }
}

/// Precedence (high to low):
/// 1. `CommitFault` -- non-checkpoint commit error.
/// 2. `StepFault` -- `YieldReason::Fault`, batch discarded.
/// 3. `RsxCheckpoint(addr)` -- `ReservedWrite("rsx")` under `FirstRsxWrite`.
/// 4. `PcReached(addr)` -- step retired the caller-supplied PC.
/// 5. `Continue`.
///
/// `callback_worker_fault_absorbed` suppresses `StepFault` so the run can resume.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::game) enum StepVerdict {
    Continue,
    CommitFault,
    StepFault,
    RsxCheckpoint(u64),
    PcReached(u64),
}

pub(in crate::game) fn classify_step_outcome(
    step: &cellgov_exec::ExecutionStepResult,
    commit_result: &Result<CommitOutcome, CommitError>,
    checkpoint: manifest::CheckpointTrigger,
    target_pc: Option<u64>,
) -> StepVerdict {
    let checkpoint_addr = rsx_write_checkpoint_addr(checkpoint, commit_result);
    if commit_result.is_err() && checkpoint_addr.is_none() {
        return StepVerdict::CommitFault;
    }
    let callback_fault_absorbed = matches!(
        commit_result,
        Ok(o) if o.callback_worker_fault_absorbed
    );
    if step.fault.is_some() && !callback_fault_absorbed {
        return StepVerdict::StepFault;
    }
    if let Some(addr) = checkpoint_addr {
        return StepVerdict::RsxCheckpoint(addr);
    }
    if let Some(target) = target_pc {
        if step.local_diagnostics.pc == Some(target) {
            return StepVerdict::PcReached(target);
        }
    }
    StepVerdict::Continue
}

#[cfg(test)]
#[path = "tests/verdict_tests.rs"]
mod tests;
