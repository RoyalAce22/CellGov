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
mod tests {
    use super::*;
    use cellgov_exec::ExecutionStepResult;
    use cellgov_mem::MemError;
    use manifest::CheckpointTrigger;

    fn ok_commit() -> Result<CommitOutcome, CommitError> {
        Ok(CommitOutcome::default())
    }

    fn rsx_checkpoint_err() -> Result<CommitOutcome, CommitError> {
        Err(CommitError::Memory(MemError::ReservedWrite {
            addr: 0xC000_0040,
            region: "rsx",
        }))
    }

    fn other_commit_err() -> Result<CommitOutcome, CommitError> {
        Err(CommitError::PayloadLengthMismatch { effect_index: 0 })
    }

    fn ok_step() -> ExecutionStepResult {
        ExecutionStepResult {
            yield_reason: cellgov_exec::YieldReason::BudgetExhausted,
            consumed_cost: cellgov_time::InstructionCost::ZERO,
            local_diagnostics: cellgov_exec::LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn faulted_step() -> ExecutionStepResult {
        ExecutionStepResult {
            yield_reason: cellgov_exec::YieldReason::Fault,
            consumed_cost: cellgov_time::InstructionCost::ZERO,
            local_diagnostics: cellgov_exec::LocalDiagnostics::empty(),
            fault: Some(cellgov_effects::FaultKind::Guest(
                cellgov_ppu::FAULT_DECODE_ERROR,
            )),
            syscall_args: None,
        }
    }

    #[test]
    fn rsx_checkpoint_fires_on_reserved_write_to_rsx() {
        assert_eq!(
            rsx_write_checkpoint_addr(CheckpointTrigger::FirstRsxWrite, &rsx_checkpoint_err()),
            Some(0xC000_0040)
        );
    }

    #[test]
    fn rsx_checkpoint_ignores_other_reserved_regions() {
        let err: Result<CommitOutcome, CommitError> =
            Err(CommitError::Memory(MemError::ReservedWrite {
                addr: 0xE000_0000,
                region: "spu_reserved",
            }));
        assert_eq!(
            rsx_write_checkpoint_addr(CheckpointTrigger::FirstRsxWrite, &err),
            None
        );
    }

    #[test]
    fn rsx_checkpoint_inert_for_process_exit_trigger() {
        assert_eq!(
            rsx_write_checkpoint_addr(CheckpointTrigger::ProcessExit, &rsx_checkpoint_err()),
            None
        );
    }

    #[test]
    fn rsx_checkpoint_ignores_successful_commit() {
        assert_eq!(
            rsx_write_checkpoint_addr(CheckpointTrigger::FirstRsxWrite, &ok_commit()),
            None
        );
    }

    #[test]
    fn rsx_checkpoint_ignores_non_memory_commit_errors() {
        assert_eq!(
            rsx_write_checkpoint_addr(CheckpointTrigger::FirstRsxWrite, &other_commit_err()),
            None
        );
    }

    #[test]
    fn classify_continue_on_clean_step_and_commit() {
        assert_eq!(
            classify_step_outcome(
                &ok_step(),
                &ok_commit(),
                CheckpointTrigger::ProcessExit,
                None
            ),
            StepVerdict::Continue
        );
    }

    #[test]
    fn classify_commit_fault_when_non_checkpoint_err() {
        assert_eq!(
            classify_step_outcome(
                &ok_step(),
                &other_commit_err(),
                CheckpointTrigger::FirstRsxWrite,
                None,
            ),
            StepVerdict::CommitFault
        );
    }

    #[test]
    fn classify_commit_fault_wins_over_step_fault() {
        assert_eq!(
            classify_step_outcome(
                &faulted_step(),
                &other_commit_err(),
                CheckpointTrigger::FirstRsxWrite,
                None,
            ),
            StepVerdict::CommitFault
        );
    }

    #[test]
    fn classify_step_fault_wins_over_pc_reached() {
        let mut s = faulted_step();
        s.local_diagnostics.pc = Some(0x1000);
        assert_eq!(
            classify_step_outcome(
                &s,
                &ok_commit(),
                CheckpointTrigger::ProcessExit,
                Some(0x1000),
            ),
            StepVerdict::StepFault
        );
    }

    #[test]
    fn classify_rsx_checkpoint_fires_under_first_rsx_write() {
        assert_eq!(
            classify_step_outcome(
                &ok_step(),
                &rsx_checkpoint_err(),
                CheckpointTrigger::FirstRsxWrite,
                None,
            ),
            StepVerdict::RsxCheckpoint(0xC000_0040)
        );
    }

    #[test]
    fn classify_rsx_checkpoint_demotes_to_commit_fault_when_trigger_mismatched() {
        assert_eq!(
            classify_step_outcome(
                &ok_step(),
                &rsx_checkpoint_err(),
                CheckpointTrigger::ProcessExit,
                None,
            ),
            StepVerdict::CommitFault
        );
    }

    #[test]
    fn classify_pc_reached_only_when_no_fault_and_no_checkpoint() {
        let mut s = ok_step();
        s.local_diagnostics.pc = Some(0x4000);
        assert_eq!(
            classify_step_outcome(
                &s,
                &ok_commit(),
                CheckpointTrigger::ProcessExit,
                Some(0x4000),
            ),
            StepVerdict::PcReached(0x4000)
        );
    }

    #[test]
    fn classify_pc_target_mismatch_continues() {
        let mut s = ok_step();
        s.local_diagnostics.pc = Some(0x4000);
        assert_eq!(
            classify_step_outcome(
                &s,
                &ok_commit(),
                CheckpointTrigger::ProcessExit,
                Some(0x5000),
            ),
            StepVerdict::Continue
        );
    }
}
