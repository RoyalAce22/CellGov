use std::collections::BTreeSet;

use strum::VariantArray;

use super::*;
use crate::exec::PpuFault;

fn unimplemented(kind: PpuInstructionKind) -> PpuFault {
    PpuFault::UnimplementedInstruction {
        kind,
        mnemonic: kind.into(),
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
    assert_eq!(unimplemented_kind(FAULT_PROGRAM_TRAP), None);
}
