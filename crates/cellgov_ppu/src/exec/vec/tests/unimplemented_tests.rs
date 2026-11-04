//! Each VX op that has no arm faults with its own mnemonic, and every VA
//! op executes.

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
fn every_recording_compare_executes() {
    let ops: Vec<VxOp> = VxOp::iter().filter(|op| op.is_vxr_compare()).collect();
    assert_eq!(ops.len(), 13);
    for op in ops {
        let insn = PpuInstruction::Vx {
            op,
            rc: true,
            vt: 3,
            va: 1,
            vb: 2,
        };
        assert_eq!(
            exec_no_mem(&insn, &mut PpuState::new()),
            ExecuteVerdict::Continue,
            "{op:?}"
        );
    }
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
fn every_va_op_executes_and_its_descriptor_admits_no_fault() {
    use crate::instruction::fuzz::PpuOutcomeClass;
    let mut executed = 0;
    for op in VaOp::iter() {
        let insn = PpuInstruction::Va {
            op,
            vt: 3,
            va: 1,
            vb: 2,
            vc: 4,
        };
        assert_eq!(
            exec_no_mem(&insn, &mut PpuState::new()),
            ExecuteVerdict::Continue,
            "{op:?}"
        );
        let raw = 0x1061_1100 | op as u32;
        assert_eq!(
            insn.fuzz_descriptor(raw).outcomes,
            [PpuOutcomeClass::Continue],
            "{op:?}"
        );
        executed += 1;
    }
    assert_eq!(executed, VaOp::iter().count());
    assert!(executed > 0);
}
