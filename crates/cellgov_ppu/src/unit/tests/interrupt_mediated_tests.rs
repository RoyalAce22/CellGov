//! The mediated external interrupt: the exception CellGov's hypervisor
//! raises for LPCR\[MER\] = 1, which a thread takes between two
//! instructions once it may.

use super::interrupt_support::*;
use super::mediated_enabled;
use crate::PpuExecutionUnit;
use cellgov_exec::{InterruptRecord, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_EE, MSR_HV, MSR_ME, MSR_PR, MSR_RI, MSR_SF};

/// Words of `nop` at [`CODE`]: more than any run here retires.
const NOPS: usize = 64;

/// A DEC value that stays non-negative for every run here.
const FAR_DEC: u32 = 0x7FFF_FFFF;

fn nops() -> GuestMemory {
    memory(&[NOP; NOPS])
}

/// A thread at [`CODE`] with MSR `msr` and a decrementer that never
/// fires here.
fn thread(msr: u64) -> PpuExecutionUnit {
    let mut unit = kernel_unit(msr);
    unit.state_mut()
        .update_supervisor(|s| {
            s.dec = FAR_DEC;
            s.dec_tb = 0;
        })
        .expect("supervisor registers");
    unit
}

/// The record of an external interrupt at `pc` taken with MSR `msr`,
/// whose SRR1 also holds `cause`.
fn external_record(pc: u64, msr: u64, cause: u64) -> InterruptRecord {
    InterruptRecord {
        vector: irq::VECTOR_EXTERNAL,
        srr0: pc,
        srr1: msr | cause,
        dsisr: None,
        dar: None,
    }
}

/// [CBE-Handbook p:276 s:9.7] A mediated external interrupt needs MSR\[EE\] = 1; [CBE-Handbook p:277 s:9.8] Table 9-27: its HSRR1 holds bits 33 and 42, which lv1 passes to the kernel's SRR1.
#[test]
fn a_mediated_exception_waits_while_ee_is_0_and_enters_0x500_with_bits_33_and_42() {
    // nop; nop; mtmsrd r7,1 with r7 = EE | RI; nop.
    let mem = memory(&[NOP, NOP, mtmsrd(7, true), NOP]);
    let mut unit = thread(KERNEL_MSR);
    unit.state_mut().set_gpr(7, MSR_EE | MSR_RI);
    assert!(unit.raise_mediated_external_exception());

    let (r, _) = run(&mut unit, &mem, 8);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(
        r.local_diagnostics.interrupt, None,
        "EE = 0 holds the exception"
    );
    assert_eq!(
        unit.retired_instructions(),
        3,
        "the MSR move ends the batch"
    );

    let (r, _) = run(&mut unit, &mem, 8);
    assert_eq!(r.yield_reason, YieldReason::InterruptBoundary);
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(external_record(
            CODE + 12,
            KERNEL_MSR | MSR_EE,
            irq::SRR1_MEDIATED_EXTERNAL
        ))
    );
    assert_eq!(
        unit.state().pc,
        CODE + 12,
        "the hypervisor moves the thread"
    );
    assert!(unit.enter_mediated_external_vector());
    assert_eq!(unit.state().pc, irq::VECTOR_EXTERNAL);
    assert_eq!(sup(&unit).msr, MSR_SF | MSR_ME);
    assert!(
        sup(&unit).mediated,
        "taking the interrupt keeps the request"
    );
    assert!(!sup(&unit).external);
}

/// [CBE-Handbook p:276 s:9.7] Mediated external interrupts are enabled when MSR\[EE\] & (^(MSR\[HV\]) | MSR\[PR\]) is 1.
#[test]
fn a_mediated_exception_is_enabled_by_ee_outside_hypervisor_state_or_in_problem_state() {
    let rows = [
        (0, false),
        (MSR_EE, true),
        (MSR_HV, false),
        (MSR_EE | MSR_HV, false),
        (MSR_PR, false),
        (MSR_EE | MSR_PR, true),
        (MSR_HV | MSR_PR, false),
        (MSR_EE | MSR_HV | MSR_PR, true),
    ];
    for (msr, enabled) in rows {
        assert_eq!(
            mediated_enabled(KERNEL_MSR | msr),
            enabled,
            "MSR {:#x}",
            KERNEL_MSR | msr
        );
    }
}

/// CellGov runs no code in hypervisor state, so the run ends at the
/// first instruction; it never reaches the external vector.
///
/// [CBE-Handbook p:276 s:9.7] Mediated external interrupts are enabled when MSR\[EE\] & (^(MSR\[HV\]) | MSR\[PR\]) is 1.
#[test]
fn a_mediated_exception_waits_in_hypervisor_state_with_ee_1() {
    let mem = nops();
    let mut unit = thread(KERNEL_MSR | MSR_EE | MSR_HV);
    assert!(unit.raise_mediated_external_exception());
    let (r, _) = run(&mut unit, &mem, 8);
    assert_ne!(r.yield_reason, YieldReason::InterruptBoundary);
    assert_eq!(r.local_diagnostics.interrupt, None);
    assert_ne!(unit.state().pc, irq::VECTOR_EXTERNAL);
    assert!(sup(&unit).mediated);
}

/// [CBE-Handbook p:276 s:9.7] There is no relative priority between direct and mediated external exceptions.
#[test]
fn a_direct_external_exception_is_taken_before_a_mediated_one() {
    let mem = nops();
    let msr = KERNEL_MSR | MSR_EE;
    let mut unit = thread(msr);
    assert!(unit.raise_external_exception());
    assert!(unit.raise_mediated_external_exception());
    let (r, _) = run(&mut unit, &mem, 8);
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(external_record(CODE, msr, 0))
    );
    assert!(sup(&unit).mediated);
}

#[test]
fn a_cleared_mediated_exception_is_not_taken() {
    let mem = nops();
    let mut unit = thread(KERNEL_MSR | MSR_EE);
    assert!(unit.raise_mediated_external_exception());
    assert!(unit.clear_mediated_external_exception());
    assert!(!sup(&unit).mediated);
    let (r, _) = run(&mut unit, &mem, 8);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(r.local_diagnostics.interrupt, None);
}

#[test]
fn a_thread_without_supervisor_registers_neither_raises_nor_clears_a_mediated_exception() {
    let mut unit = PpuExecutionUnit::new(cellgov_event::UnitId::new(0));
    assert!(!unit.raise_mediated_external_exception());
    assert!(!unit.clear_mediated_external_exception());
}
