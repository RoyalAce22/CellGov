//! `vx_executes` against `execute_vx` and the fuzz descriptor, for every
//! VX op in both record forms.

use crate::exec::test_support::exec_no_mem;
use crate::exec::{vx_executes, ExecuteVerdict};
use crate::instruction::fuzz::PpuOutcomeClass;
use crate::instruction::ops::VxOp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use strum::IntoEnumIterator;

#[test]
fn every_vx_op_continues_exactly_when_vx_executes_says_so() {
    let mut continuing = 0;
    for op in VxOp::iter() {
        for rc in [false, true] {
            let insn = PpuInstruction::Vx {
                op,
                rc,
                vt: 3,
                va: 1,
                vb: 2,
            };
            let continues = exec_no_mem(&insn, &mut PpuState::new()) == ExecuteVerdict::Continue;
            assert_eq!(continues, vx_executes(op, rc), "{op:?} rc={rc}");
            let raw = 0x1061_1000 | op as u32 | u32::from(rc) << 10;
            let outcomes = insn.fuzz_descriptor(raw).outcomes;
            assert_eq!(
                outcomes == [PpuOutcomeClass::Continue],
                continues,
                "{op:?} rc={rc}: {outcomes:?}"
            );
            continuing += usize::from(continues);
        }
    }
    // Each op without Rc but the ten whose leaves are open, and each
    // compare with it.
    let compares = VxOp::iter().filter(|op| op.is_vxr_compare()).count();
    assert_eq!(continuing, VxOp::iter().count() - 10 + compares);
}

#[test]
fn vaddfp_and_vsubfp_execute_without_a_record_form() {
    for op in [VxOp::Vaddfp, VxOp::Vsubfp] {
        assert!(vx_executes(op, false), "{op:?}");
        assert!(!vx_executes(op, true), "{op:?}");
    }
    assert!(vx_executes(VxOp::Vcmpeqfp, true));
    assert!(!vx_executes(VxOp::Vrefp, false));
}
