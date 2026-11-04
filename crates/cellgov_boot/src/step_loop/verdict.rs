//! The verdict precedence both step loops classify a step by.

use cellgov_core::{CommitError, CommitOutcome};

use crate::manifest;

pub(crate) fn rsx_write_checkpoint_addr(
    trigger: manifest::CheckpointTrigger,
    commit_result: &Result<CommitOutcome, CommitError>,
) -> Option<u64> {
    commit_result
        .as_ref()
        .err()
        .and_then(|e| rsx_checkpoint_addr(trigger, *e))
}

/// The reserved-RSX address in `error`, when `trigger` stops there.
///
/// A caller that drives the runtime outside these loops -- schedule
/// exploration, say -- sees a checkpoint write that reached the commit
/// as an ordinary commit refusal. This call tells the title's declared
/// stop apart from any other refusal; [`rsx_reserved_write`] does the
/// same for a PPU store refused at the store.
pub fn rsx_checkpoint_addr(
    trigger: manifest::CheckpointTrigger,
    error: CommitError,
) -> Option<u64> {
    match error {
        CommitError::Memory(cellgov_mem::MemError::ReservedWrite { addr, region }) => {
            rsx_reserved_write(trigger, addr, region)
        }
        _ => None,
    }
}

/// `addr` when a write refused by `region` is the stop `trigger` names:
/// the RSX region or the context window's control block, under
/// `FirstRsxWrite`.
pub fn rsx_reserved_write(
    trigger: manifest::CheckpointTrigger,
    addr: u64,
    region: &str,
) -> Option<u64> {
    (trigger == manifest::CheckpointTrigger::FirstRsxWrite
        && (region == "rsx" || region == crate::prepare::RSX_CONTROL_REGION))
        .then_some(addr)
}

/// The reserved-RSX address a PPU store in `step` faulted on, when
/// `trigger` stops there. A PPU store faults at the store, so the
/// checkpoint arrives as a step fault.
fn rsx_store_checkpoint_addr(
    trigger: manifest::CheckpointTrigger,
    step: &cellgov_exec::ExecutionStepResult,
) -> Option<u64> {
    let diag = &step.local_diagnostics;
    match (diag.data_access, diag.faulting_ea) {
        (
            Some(cellgov_exec::DataAccessFault {
                store: true,
                refused_by: Some(region),
            }),
            Some(addr),
        ) if step.fault.is_some() => rsx_reserved_write(trigger, addr, region),
        _ => None,
    }
}

/// Precedence (high to low):
/// 1. `CommitFault` -- non-checkpoint commit error.
/// 2. `StepFault` -- `YieldReason::Fault`, batch discarded, other than
///    a PPU store fault on the checkpoint's region.
/// 3. `RsxCheckpoint(addr)` -- a reserved write to the RSX region or the
///    context window's control block under `FirstRsxWrite`, refused at
///    the store or at the commit.
/// 4. `PcReached(addr)` -- step retired the caller-supplied PC.
/// 5. `Continue`.
///
/// Only the checkpoint's own store fault suppresses `StepFault`: any
/// other discarded batch ends the boot, whichever unit emitted it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StepVerdict {
    Continue,
    CommitFault,
    StepFault,
    RsxCheckpoint(u64),
    PcReached(u64),
}

pub(crate) fn classify_step_outcome(
    step: &cellgov_exec::ExecutionStepResult,
    commit_result: &Result<CommitOutcome, CommitError>,
    checkpoint: manifest::CheckpointTrigger,
    target_pc: Option<u64>,
) -> StepVerdict {
    let commit_checkpoint = rsx_write_checkpoint_addr(checkpoint, commit_result);
    if commit_result.is_err() && commit_checkpoint.is_none() {
        return StepVerdict::CommitFault;
    }
    let store_checkpoint = rsx_store_checkpoint_addr(checkpoint, step);
    if step.fault.is_some() && store_checkpoint.is_none() {
        return StepVerdict::StepFault;
    }
    let checkpoint_addr = commit_checkpoint.or(store_checkpoint);
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

#[cfg(test)]
#[path = "tests/verdict_control_tests.rs"]
mod control_tests;

#[cfg(test)]
#[path = "tests/verdict_store_tests.rs"]
mod store_tests;
