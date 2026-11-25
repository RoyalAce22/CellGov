//! A thread with supervisor registers reads BP_VR in supervisor state and
//! takes a program interrupt for the read in problem state.

use super::interrupt_support::*;
use cellgov_exec::{InterruptRecord, YieldReason};
use cellgov_ps3_abi::hw::ppu::CELL_BP_VR;
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_PR, SPR_BP_VR};

#[test]
fn a_supervisor_state_bp_vr_read_retires_and_takes_no_interrupt() {
    let mem = memory(&[mfspr(3, SPR_BP_VR), NOP]);
    let mut unit = kernel_unit(KERNEL_MSR);
    let (r, _) = run(&mut unit, &mem, 2);
    assert_eq!(r.fault, None);
    assert_eq!(r.local_diagnostics.interrupt, None);
    assert_ne!(r.yield_reason, YieldReason::InterruptBoundary);
    assert_eq!(unit.retired_instructions(), 2);
    assert_eq!(unit.state().gpr[3], u64::from(CELL_BP_VR));
}

/// [PPC-Book3 p:71 s:5.5.9] A program interrupt sets SRR0 to the instruction that caused it.
/// [PPC-Book3 p:72 s:5.5.9] A program interrupt sets SRR1 bit 45 for a privileged instruction.
/// [CBE-Handbook p:254 s:9.5.9] The PPE takes a privileged-instruction interrupt for a problem-state access to a supervisor SPR.
#[test]
fn a_problem_state_bp_vr_read_takes_the_privileged_program_interrupt() {
    let msr = KERNEL_MSR | MSR_PR;
    let mem = memory(&[mfspr(3, SPR_BP_VR)]);
    let mut unit = kernel_unit(msr);
    unit.state_mut().set_gpr(3, 0x1234);
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(r.yield_reason, YieldReason::InterruptBoundary);
    assert_eq!(r.fault, None);
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(InterruptRecord {
            vector: irq::VECTOR_PROGRAM,
            srr0: CODE,
            srr1: msr | irq::SRR1_PROGRAM_PRIVILEGED,
            dsisr: None,
            dar: None,
        })
    );
    assert_eq!(unit.state().pc, irq::VECTOR_PROGRAM);
    assert_eq!(unit.state().gpr[3], 0x1234);
}
