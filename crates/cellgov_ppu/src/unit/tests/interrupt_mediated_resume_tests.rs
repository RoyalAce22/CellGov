//! The end of a mediated take that CellGov's hypervisor answers with no
//! vector entry: lv1's `hrfid` returns the thread to the interrupted
//! instruction.

use super::hrfid_msr;
use super::interrupt_support::*;
use crate::PpuExecutionUnit;
use cellgov_exec::YieldReason;
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_ps3_abi::hw::ppu_supervisor::{
    MSR_BIT1, MSR_DR, MSR_EE, MSR_IR, MSR_ME, MSR_PR, MSR_RI, MSR_SF,
};

/// SRR0 and SRR1 before the take: values that no take writes.
const OLD_SRR0: u64 = 0x0000_0000_0001_2340;
const OLD_SRR1: u64 = 0x8000_0000_0000_1032;

/// A thread that sets EE with a mediated exception raised, and then takes
/// it ahead of the word after its `mtmsrd`.
fn taken() -> PpuExecutionUnit {
    let mem = memory(&[mtmsrd(7, true), NOP, NOP, NOP]);
    let mut unit = kernel_unit(KERNEL_MSR);
    unit.state_mut()
        .update_supervisor(|s| {
            s.dec = 0x7FFF_FFFF;
            s.dec_tb = 0;
            s.srr0 = OLD_SRR0;
            s.srr1 = OLD_SRR1;
        })
        .expect("supervisor registers");
    unit.state_mut().set_gpr(7, MSR_EE | MSR_RI);
    assert!(unit.raise_mediated_external_exception());
    let (r, _) = run(&mut unit, &mem, 8);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    let (r, _) = run(&mut unit, &mem, 8);
    assert_eq!(r.yield_reason, YieldReason::InterruptBoundary);
    assert_eq!(
        r.local_diagnostics.interrupt.map(|i| (i.srr0, i.srr1)),
        Some((CODE + 4, KERNEL_MSR | MSR_EE | irq::SRR1_MEDIATED_EXTERNAL))
    );
    unit
}

/// lv1 reloads HSRR0, HSRR1, SRR0 and SRR1 from its entry frame
/// [OFW-4.93 lv1 a:0x1AF8-0x1B10] [OFW-4.93 lv1 a:0x1B2C-0x1B40], and its
/// `hrfid` returns to HSRR0 [OFW-4.93 lv1 a:0x1B84].
#[test]
fn a_resumed_take_keeps_the_pc_msr_srr0_and_srr1_of_the_interrupted_thread() {
    let mut unit = taken();
    assert_eq!(unit.state().pc, CODE + 4);
    assert!(unit.resume_from_mediated_take());
    assert_eq!(unit.state().pc, CODE + 4);
    assert_eq!(sup(&unit).msr, KERNEL_MSR | MSR_EE);
    assert_eq!((sup(&unit).srr0, sup(&unit).srr1), (OLD_SRR0, OLD_SRR1));
}

#[test]
fn a_resumed_thread_runs_the_interrupted_instruction_next() {
    let mut unit = taken();
    assert!(unit.clear_mediated_external_exception());
    assert!(unit.resume_from_mediated_take());
    let retired = unit.retired_instructions();
    let (r, _) = run(&mut unit, &memory(&[mtmsrd(7, true), NOP, NOP, NOP]), 2);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(r.local_diagnostics.interrupt, None);
    assert_eq!(unit.retired_instructions(), retired + 2);
    assert_eq!(unit.state().pc, CODE + 12);
}

#[test]
fn an_entered_take_sets_srr0_srr1_and_the_vector_msr() {
    let mut unit = taken();
    assert!(unit.enter_mediated_external_vector());
    assert_eq!(unit.state().pc, irq::VECTOR_EXTERNAL);
    assert_eq!(sup(&unit).msr, MSR_SF | MSR_ME);
    assert_eq!(
        (sup(&unit).srr0, sup(&unit).srr1),
        (CODE + 4, KERNEL_MSR | MSR_EE | irq::SRR1_MEDIATED_EXTERNAL)
    );
}

/// [PPC-Book3 p:13 s:2.3] hrfid places the OR of HSRR1 bits 0 and 1 into MSR\[SF\], the OR of HSRR1 bit 49 with bits 48, 58 and 59 into MSR\[EE\], MSR\[IR\] and MSR\[DR\], and HSRR1 bits 1:32, 37:41, 49:57 and 60:63 into the same MSR bits.
#[test]
fn hrfid_takes_the_msr_from_hsrr1_and_keeps_the_interrupt_information_bits() {
    let info = irq::SRR1_MEDIATED_EXTERNAL;
    let rows = [
        (KERNEL_MSR | MSR_EE | info, 0, KERNEL_MSR | MSR_EE),
        (MSR_BIT1 | MSR_EE, 0, MSR_SF | MSR_BIT1 | MSR_EE),
        (
            MSR_SF | MSR_PR,
            0,
            MSR_SF | MSR_PR | MSR_EE | MSR_IR | MSR_DR,
        ),
        (MSR_SF | MSR_EE | info, info, MSR_SF | MSR_EE | info),
    ];
    for (hsrr1, msr, expected) in rows {
        assert_eq!(
            hrfid_msr(hsrr1, msr),
            expected,
            "HSRR1 {hsrr1:#x} MSR {msr:#x}"
        );
    }
}

#[test]
fn a_thread_without_supervisor_registers_neither_enters_nor_resumes_a_mediated_take() {
    let mut unit = PpuExecutionUnit::new(cellgov_event::UnitId::new(0));
    assert!(!unit.enter_mediated_external_vector());
    assert!(!unit.resume_from_mediated_take());
}
