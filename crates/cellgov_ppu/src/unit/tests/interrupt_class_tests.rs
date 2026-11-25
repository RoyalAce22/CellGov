//! A thread with supervisor registers takes each instruction-caused
//! interrupt at its vector, with the class's SRR0, SRR1, DSISR and DAR.

use super::interrupt_support::*;
use crate::PpuExecutionUnit;
use cellgov_exec::{ExecutionStepResult, ExecutionUnit, InterruptRecord, UnitStatus, YieldReason};
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_IR, MSR_ME, MSR_PR, MSR_SF};

/// The MSR at every vector here: [`KERNEL_MSR`] after interrupt entry
/// clears FP, RI, PR, IR and DR.
///
/// [CBE-Handbook p:292 s:9.12] Table 9-40: interrupt entry clears FP, PR, IR, DR and RI, sets SF and keeps ME.
const ENTRY_MSR: u64 = MSR_SF | MSR_ME;

/// A DSISR and DAR that no interrupt here sets, so a test sees which
/// interrupt wrote them.
const STALE_DSISR: u32 = 0x0000_1234;
const STALE_DAR: u64 = 0x0000_5678;

/// A thread with MSR `msr` whose DSISR and DAR hold the stale values.
fn thread(msr: u64) -> PpuExecutionUnit {
    let mut unit = kernel_unit(msr);
    unit.state_mut()
        .update_supervisor(|s| {
            s.dsisr = STALE_DSISR;
            s.dar = STALE_DAR;
        })
        .expect("supervisor registers");
    unit
}

/// Asserts that `r` yielded at the interrupt boundary with `want`, and
/// that the thread holds `want`'s registers, [`ENTRY_MSR`] and the vector.
fn assert_delivered(unit: &PpuExecutionUnit, r: &ExecutionStepResult, want: InterruptRecord) {
    assert_eq!(
        r.yield_reason,
        YieldReason::InterruptBoundary,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(r.fault, None);
    assert_eq!(r.syscall_args, None);
    assert_eq!(r.local_diagnostics.interrupt, Some(want));
    assert_eq!(unit.status(), UnitStatus::Runnable);
    let s = sup(unit);
    assert_eq!((s.srr0, s.srr1), (want.srr0, want.srr1));
    assert_eq!(s.dsisr, want.dsisr.unwrap_or(STALE_DSISR));
    assert_eq!(s.dar, want.dar.unwrap_or(STALE_DAR));
    assert_eq!(s.msr, ENTRY_MSR);
    assert_eq!(unit.state().pc, want.vector);
}

/// [PPC-Book3 p:67 s:5.5.3] A data storage interrupt sets SRR0 to the faulting instruction, DSISR bit 1 for no translation and bit 6 for a store, and DAR to the EA.
#[test]
fn a_data_storage_interrupt_vectors_with_srr0_dsisr_and_dar() {
    let ea = SEGMENT + 0x3000;
    for (insn, dsisr) in [
        (lwz(4, 0, 5), irq::DSISR_NOT_FOUND),
        (stw(4, 0, 5), irq::DSISR_NOT_FOUND | irq::DSISR_STORE),
    ] {
        let mem = memory(&[insn]);
        let mut unit = thread(KERNEL_MSR | MSR_DR);
        unit.state_mut().set_gpr(5, ea);
        let (r, effects) = run(&mut unit, &mem, 1);
        assert_delivered(
            &unit,
            &r,
            InterruptRecord {
                vector: irq::VECTOR_DATA_STORAGE,
                srr0: CODE,
                srr1: KERNEL_MSR | MSR_DR,
                dsisr: Some(dsisr),
                dar: Some(ea),
            },
        );
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, cellgov_effects::Effect::SharedWriteIntent { .. })),
            "insn {insn:#x}: the refused store publishes nothing"
        );
    }
}

/// [PPC-Book3 p:68 s:5.5.4] A data segment interrupt sets SRR0 to the faulting instruction and DAR to the EA; [CBE-Handbook p:251 s:9.5.4] the PPE sets DSISR bit 10.
#[test]
fn a_data_segment_interrupt_vectors_with_dsisr_bit_10_and_dar() {
    let mem = memory(&[lwz(4, 0, 5)]);
    let mut unit = thread(KERNEL_MSR | MSR_DR);
    unit.state_mut().set_gpr(5, NO_SEGMENT);
    let (r, _) = run(&mut unit, &mem, 1);
    assert_delivered(
        &unit,
        &r,
        InterruptRecord {
            vector: irq::VECTOR_DATA_SEGMENT,
            srr0: CODE,
            srr1: KERNEL_MSR | MSR_DR,
            dsisr: Some(irq::DSISR_SEGMENT),
            dar: Some(NO_SEGMENT),
        },
    );
}

/// [PPC-Book3 p:68 s:5.5.5] An instruction storage interrupt sets SRR0 to the instruction the processor would run, and SRR1 bit 33 when no translation is found.
#[test]
fn an_instruction_storage_interrupt_vectors_with_srr1_bit_33_and_the_msr_copy() {
    let pc = SEGMENT + 0x3000;
    let mem = memory(&[]);
    let mut unit = thread(KERNEL_MSR | MSR_IR);
    unit.state_mut().pc = pc;
    let (r, _) = run(&mut unit, &mem, 1);
    assert_delivered(
        &unit,
        &r,
        InterruptRecord {
            vector: irq::VECTOR_INSTRUCTION_STORAGE,
            srr0: pc,
            srr1: KERNEL_MSR | MSR_IR | irq::SRR1_FETCH_NOT_FOUND,
            dsisr: None,
            dar: None,
        },
    );
}

/// [PPC-Book3 p:69 s:5.5.6] An instruction segment interrupt sets SRR0 to the instruction the processor would run, and SRR1 bits 33:36 and 42:47 to 0.
#[test]
fn an_instruction_segment_interrupt_vectors_with_the_msr_copy_alone() {
    let mem = memory(&[]);
    let mut unit = thread(KERNEL_MSR | MSR_IR);
    unit.state_mut().pc = NO_SEGMENT;
    let (r, _) = run(&mut unit, &mem, 1);
    assert_delivered(
        &unit,
        &r,
        InterruptRecord {
            vector: irq::VECTOR_INSTRUCTION_SEGMENT,
            srr0: NO_SEGMENT,
            srr1: KERNEL_MSR | MSR_IR,
            dsisr: None,
            dar: None,
        },
    );
}

/// [PPC-Book3 p:70 s:5.5.8] An alignment interrupt sets DAR to the EA; [CBE-Handbook p:253 s:9.5.8] the PPE does not modify DSISR.
#[test]
fn an_alignment_interrupt_vectors_with_dar_and_leaves_dsisr() {
    let mem = memory(&[lwarx(4, 0, 5)]);
    let mut unit = thread(KERNEL_MSR);
    unit.state_mut().set_gpr(5, DATA + 2);
    let (r, _) = run(&mut unit, &mem, 1);
    assert_delivered(
        &unit,
        &r,
        InterruptRecord {
            vector: irq::VECTOR_ALIGNMENT,
            srr0: CODE,
            srr1: KERNEL_MSR,
            dsisr: None,
            dar: Some(DATA + 2),
        },
    );
}

/// [PPC-Book3 p:72 s:5.5.9] A program interrupt sets SRR0 to the instruction that caused it, and SRR1 bit 44 for an illegal instruction, 45 for a privileged one and 46 for a trap.
#[test]
fn a_program_interrupt_vectors_with_its_cause_bit_and_the_msr_copy() {
    let cases = [
        ("illegal", ILLEGAL, KERNEL_MSR, irq::SRR1_PROGRAM_ILLEGAL),
        (
            "privileged",
            mfmsr(3),
            KERNEL_MSR | MSR_PR,
            irq::SRR1_PROGRAM_PRIVILEGED,
        ),
        ("trap", TRAP, KERNEL_MSR, irq::SRR1_PROGRAM_TRAP),
    ];
    for (name, insn, msr, cause) in cases {
        let mem = memory(&[insn]);
        let mut unit = thread(msr);
        let (r, _) = run(&mut unit, &mem, 1);
        let want = InterruptRecord {
            vector: irq::VECTOR_PROGRAM,
            srr0: CODE,
            srr1: msr | cause,
            dsisr: None,
            dar: None,
        };
        assert_eq!(r.local_diagnostics.interrupt, Some(want), "{name}");
        assert_delivered(&unit, &r, want);
    }
}

/// [PPC-Book3 p:73 s:5.5.13] A system call interrupt sets SRR0 to the instruction after `sc`, and SRR1 bits 33:36 and 42:47 to 0.
#[test]
fn a_system_call_vectors_to_0xc00_after_the_sc_retires_and_yields_no_syscall() {
    let mem = memory(&[SC]);
    let mut unit = thread(KERNEL_MSR);
    let (r, _) = run(&mut unit, &mem, 4);
    assert_delivered(
        &unit,
        &r,
        InterruptRecord {
            vector: irq::VECTOR_SYSTEM_CALL,
            srr0: CODE + 4,
            srr1: KERNEL_MSR,
            dsisr: None,
            dar: None,
        },
    );
    assert_eq!(unit.retired_instructions(), 1, "the sc retires");
    assert_eq!(r.local_diagnostics.syscall_lev, None);
}
