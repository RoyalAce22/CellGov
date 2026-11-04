//! Each VX and VA op that has no arm faults with its own mnemonic.

use strum::IntoEnumIterator;

use crate::exec::test_support::exec_no_mem;
use crate::exec::{ExecuteVerdict, PpuFault};
use crate::instruction::ops::{VaOp, VxOp};
use crate::instruction::{PpuInstruction, PpuInstructionKind};
use crate::state::PpuState;

/// The kind and mnemonic of an unimplemented-instruction fault, or `None`
/// for any other verdict.
fn unimplemented(verdict: ExecuteVerdict) -> Option<(PpuInstructionKind, &'static str)> {
    match verdict {
        ExecuteVerdict::Fault(PpuFault::UnimplementedInstruction { kind, mnemonic }) => {
            Some((kind, mnemonic))
        }
        _ => None,
    }
}

#[test]
fn an_unimplemented_vx_op_names_itself() {
    let mut faulted = 0;
    for op in VxOp::iter() {
        let insn = PpuInstruction::Vx {
            op,
            rc: false,
            vt: 3,
            va: 1,
            vb: 2,
        };
        if let Some(named) = unimplemented(exec_no_mem(&insn, &mut PpuState::new())) {
            assert_eq!(named, (PpuInstructionKind::Vx, op.into()), "{op:?}");
            faulted += 1;
        }
    }
    assert!(faulted > 0);
}

#[test]
fn an_unimplemented_recording_compare_names_its_record_form() {
    let mut faulted = 0;
    for op in VxOp::iter().filter(|op| op.is_vxr_compare()) {
        let insn = PpuInstruction::Vx {
            op,
            rc: true,
            vt: 3,
            va: 1,
            vb: 2,
        };
        if let Some((kind, mnemonic)) = unimplemented(exec_no_mem(&insn, &mut PpuState::new())) {
            let base: &'static str = op.into();
            assert_eq!(kind, PpuInstructionKind::Vx, "{op:?}");
            assert_eq!(mnemonic, format!("{base}."), "{op:?}");
            faulted += 1;
        }
    }
    assert!(faulted > 0);
}

#[test]
fn a_record_form_of_a_vx_op_with_no_record_form_names_the_op() {
    let insn = PpuInstruction::Vx {
        op: VxOp::Vaddubm,
        rc: true,
        vt: 3,
        va: 1,
        vb: 2,
    };
    assert_eq!(
        unimplemented(exec_no_mem(&insn, &mut PpuState::new())),
        Some((PpuInstructionKind::Vx, "vaddubm"))
    );
}

#[test]
fn an_unimplemented_va_op_names_itself() {
    let mut faulted = 0;
    for op in VaOp::iter() {
        let insn = PpuInstruction::Va {
            op,
            vt: 3,
            va: 1,
            vb: 2,
            vc: 4,
        };
        if let Some(named) = unimplemented(exec_no_mem(&insn, &mut PpuState::new())) {
            assert_eq!(named, (PpuInstructionKind::Va, op.into()), "{op:?}");
            faulted += 1;
        }
    }
    assert!(faulted > 0);
}
