use std::collections::BTreeSet;

use strum::VariantArray;

use super::*;
use crate::exec::{PpuFault, PpuInterrupt, ProgramCause};

fn unimplemented(kind: PpuInstructionKind) -> PpuFault {
    PpuFault::UnimplementedInstruction {
        kind,
        mnemonic: kind.into(),
    }
}

/// A fault of `class` and the guest code it must yield.
fn sample(class: PpuFaultClass) -> (PpuFault, u32) {
    use PpuFaultClass as C;
    let interrupt = PpuFault::Interrupt;
    match class {
        C::InstructionStorage => (
            interrupt(PpuInterrupt::InstructionStorage { no_execute: false }),
            0x0102_4000,
        ),
        C::InstructionSegment => (interrupt(PpuInterrupt::InstructionSegment), 0x0111_0000),
        C::DataStorage => (
            interrupt(PpuInterrupt::DataStorage {
                ea: 0x10,
                store: true,
                protection: true,
            }),
            0x0106_0A00,
        ),
        C::DataSegment => (
            interrupt(PpuInterrupt::DataSegment {
                ea: 0x10,
                store: false,
            }),
            0x0110_0020,
        ),
        C::Alignment => (PpuFault::alignment(0x11, true), 0x010B_0000),
        C::ProgramFpEnabled => (
            interrupt(PpuInterrupt::Program(ProgramCause::FpEnabled)),
            0x010A_1000,
        ),
        C::ProgramIllegal => (PpuFault::illegal("x"), 0x010A_0800),
        C::ProgramPrivileged => (PpuFault::privileged("x"), 0x010A_0400),
        C::ProgramTrap => (PpuFault::trap(0x1F), 0x010A_021F),
        C::FpUnavailable => (interrupt(PpuInterrupt::FpUnavailable), 0x0112_0000),
        C::VxuUnavailable => (interrupt(PpuInterrupt::VxuUnavailable), 0x0113_0000),
        C::SystemCall => (interrupt(PpuInterrupt::SystemCall { lev: 1 }), 0x010F_0001),
        C::Unimplemented => (unimplemented(PpuInstructionKind::Vx), {
            0x0109_0000 | PpuInstructionKind::Vx as u32
        }),
        C::DebugBreak => (PpuFault::DebugBreak, 0x0108_0000),
        C::Internal => (PpuFault::Internal("x"), 0x0114_0000),
    }
}

#[test]
fn every_class_yields_its_pinned_code_and_decodes_back_to_itself() {
    for &class in PpuFaultClass::VARIANTS {
        let (fault, want) = sample(class);
        assert_eq!(fault.class(), class, "{fault:?}");
        assert_eq!(fault.guest_code(), want, "{class:?}");
        assert_eq!(PpuFaultClass::of_code(want), Some(class), "{class:?}");
    }
}

/// The program interrupt's four reasons share its prefix and differ in
/// the reason byte; every other class has a prefix of its own.
#[test]
fn no_two_classes_share_a_prefix() {
    let keys: BTreeSet<u32> = PpuFaultClass::VARIANTS
        .iter()
        .map(|&class| {
            let code = sample(class).0.guest_code();
            if code & 0xFFFF_0000 == FAULT_PROGRAM {
                code & 0xFFFF_FF00
            } else {
                code & 0xFFFF_0000
            }
        })
        .collect();
    assert_eq!(keys.len(), PpuFaultClass::VARIANTS.len());
}

#[test]
fn a_program_code_names_one_reason_or_none() {
    assert_eq!(PpuFaultClass::of_code(FAULT_PROGRAM), None);
    assert_eq!(PpuFaultClass::of_code(FAULT_PROGRAM | 0x0C00), None);
    assert_eq!(PpuFaultClass::of_code(FAULT_PROGRAM | 0x0100), None);
    assert_eq!(PpuFaultClass::of_code(0x0105_0000), None);
}

/// [PPC-Book3 p:72 s:5.5.9] A program interrupt sets one of SRR1 bits 43 to 46 and clears bit 47 when SRR0 addresses the instruction that caused it.
#[test]
fn every_program_reason_sets_exactly_one_of_srr1_bits_43_to_46() {
    let reasons = [
        ProgramCause::FpEnabled,
        ProgramCause::Illegal("x"),
        ProgramCause::Privileged("x"),
        ProgramCause::Trap { to: 0x1F },
    ];
    let mut seen = 0u64;
    for reason in reasons {
        let srr1 = PpuInterrupt::Program(reason).record(0).srr1;
        let bits_43_to_47 = (srr1 >> (63 - 47)) & 0x1F;
        assert_eq!(bits_43_to_47.count_ones(), 1, "{reason:?}: {srr1:#x}");
        assert_eq!(bits_43_to_47 & 1, 0, "{reason:?}: bit 47");
        assert_eq!(srr1 & !(0x1E << (63 - 47)), 0, "{reason:?}: {srr1:#x}");
        seen |= srr1;
    }
    assert_eq!(seen.count_ones(), 4, "each reason has its own bit");
}

#[test]
fn every_class_but_the_cellgov_stops_is_an_interrupt() {
    for &class in PpuFaultClass::VARIANTS {
        let (fault, _) = sample(class);
        assert_eq!(
            class.is_interrupt(),
            fault.interrupt().is_some(),
            "{class:?}"
        );
    }
}

#[test]
fn every_kind_has_its_own_unimplemented_code() {
    let codes: BTreeSet<u32> = PpuInstructionKind::VARIANTS
        .iter()
        .map(|&kind| unimplemented(kind).guest_code())
        .collect();
    assert_eq!(codes.len(), PpuInstructionKind::VARIANTS.len());
}

#[test]
fn an_unimplemented_code_names_its_kind() {
    for &kind in PpuInstructionKind::VARIANTS {
        let code = unimplemented(kind).guest_code();
        assert_eq!(code & 0xFFFF_0000, FAULT_UNIMPLEMENTED_INSN);
        assert_eq!(unimplemented_kind(code), Some(kind), "{kind:?}");
    }
}

#[test]
fn a_code_of_another_class_or_past_the_last_kind_names_no_kind() {
    let past = PpuInstructionKind::VARIANTS.len() as u32;
    assert_eq!(unimplemented_kind(FAULT_UNIMPLEMENTED_INSN | past), None);
    assert_eq!(unimplemented_kind(FAULT_PROGRAM), None);
}
