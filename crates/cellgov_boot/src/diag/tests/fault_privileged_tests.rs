//! The fault report for a privileged instruction names the fault class
//! and the instruction word at the faulting PC.

use super::format_fault;
use crate::step_loop::PcRing;
use cellgov_core::Runtime;
use cellgov_effects::FaultKind;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionStepResult, LocalDiagnostics, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::{Budget, InstructionCost};

/// `mfmsr r3`.
const MFMSR: u32 = 0x7C60_00A6;

#[test]
fn a_privileged_instruction_names_its_class_and_word() {
    let mut mem = GuestMemory::new(0x1000);
    let range = ByteRange::new(GuestAddr::new(0x10), 4).unwrap();
    mem.apply_commit(range, &MFMSR.to_be_bytes()).unwrap();
    let rt = Runtime::new(mem, Budget::new(1), 100);
    let fault = FaultKind::Guest(cellgov_ppu::FAULT_PRIVILEGED_INSTRUCTION);
    let result = ExecutionStepResult {
        yield_reason: YieldReason::Fault,
        consumed_cost: InstructionCost::new(0),
        local_diagnostics: LocalDiagnostics::with_pc(0x10),
        fault: Some(fault),
        syscall_args: None,
    };
    let out = format_fault(&rt, UnitId::new(0), &result, &fault, 7, &PcRing::new(), &[]);
    assert!(
        out.contains("PRIVILEGED_INSTRUCTION at PC=0x00000010 (raw=0x7c6000a6)"),
        "got {out}"
    );
}
