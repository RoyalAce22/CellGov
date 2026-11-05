//! A PPU store that faults at the store, classified against the RSX
//! checkpoint.

use super::*;
use cellgov_exec::{DataAccessFault, ExecutionStepResult};
use manifest::CheckpointTrigger;

fn store_fault(addr: u64, refused_by: Option<&'static str>) -> ExecutionStepResult {
    let mut diag = cellgov_exec::LocalDiagnostics::empty();
    diag.pc = Some(0x1_0000);
    diag.faulting_ea = Some(addr);
    diag.data_access = Some(DataAccessFault {
        store: true,
        refused_by,
    });
    // The DSISR store bit, plus the protection bit for a refusing region
    // or the no-translation bit for an unmapped address.
    let detail = if refused_by.is_some() { 0x0A00 } else { 0x4200 };
    ExecutionStepResult {
        yield_reason: cellgov_exec::YieldReason::Fault,
        consumed_cost: cellgov_time::InstructionCost::ZERO,
        local_diagnostics: diag,
        fault: Some(cellgov_effects::FaultKind::Guest(
            cellgov_ppu::FAULT_DATA_STORAGE | detail,
        )),
        syscall_args: None,
    }
}

fn classify(step: &ExecutionStepResult, trigger: CheckpointTrigger) -> StepVerdict {
    classify_step_outcome(step, &Ok(CommitOutcome::default()), trigger, None)
}

#[test]
fn a_store_refused_by_the_rsx_region_is_the_checkpoint() {
    assert_eq!(
        classify(
            &store_fault(0xC000_0040, Some("rsx")),
            CheckpointTrigger::FirstRsxWrite
        ),
        StepVerdict::RsxCheckpoint(0xC000_0040)
    );
}

#[test]
fn a_store_refused_by_the_control_block_is_the_checkpoint() {
    let region = crate::prepare::RSX_CONTROL_REGION;
    assert_eq!(
        classify(
            &store_fault(0x4000_0040, Some(region)),
            CheckpointTrigger::FirstRsxWrite
        ),
        StepVerdict::RsxCheckpoint(0x4000_0040)
    );
}

#[test]
fn a_store_refused_by_the_rsx_region_faults_under_another_trigger() {
    assert_eq!(
        classify(
            &store_fault(0xC000_0040, Some("rsx")),
            CheckpointTrigger::ProcessExit
        ),
        StepVerdict::StepFault
    );
}

#[test]
fn a_store_refused_by_another_region_faults() {
    assert_eq!(
        classify(
            &store_fault(0xE000_0000, Some("spu_reserved")),
            CheckpointTrigger::FirstRsxWrite
        ),
        StepVerdict::StepFault
    );
}

#[test]
fn a_store_to_an_unmapped_address_faults() {
    assert_eq!(
        classify(
            &store_fault(0xC000_0040, None),
            CheckpointTrigger::FirstRsxWrite
        ),
        StepVerdict::StepFault
    );
}

#[test]
fn a_load_record_never_names_the_checkpoint() {
    let mut step = store_fault(0xC000_0040, Some("rsx"));
    step.local_diagnostics.data_access = Some(DataAccessFault {
        store: false,
        refused_by: Some("rsx"),
    });
    assert_eq!(
        classify(&step, CheckpointTrigger::FirstRsxWrite),
        StepVerdict::StepFault
    );
}
