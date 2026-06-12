//! Step-verdict classification and RSX-write checkpoint detection.

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
