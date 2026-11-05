//! A fused `or; add` issues the ops of its two halves: the `or`, or its
//! quickened `mr` when RS equals RB, then the `add`.

use super::classify;
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;

/// The single op `insn` classifies to.
fn one_op(insn: &I) -> super::Op {
    let (ops, n) = classify(insn, &PpuState::new());
    assert_eq!(n, 1);
    ops[0]
}

/// The ops a fused `or r3, rS, rB; add r6, r8, r7` classifies to. Every
/// operand field names its own register, so a swapped field changes an op.
fn fused(rs: u8, rb: u8) -> Vec<super::Op> {
    let pair = I::OrAdd {
        ra: 3,
        rs,
        rb,
        rt: 6,
        add_ra: 8,
        add_rb: 7,
    };
    let (ops, n) = classify(&pair, &PpuState::new());
    ops[..n].to_vec()
}

fn add() -> I {
    I::Add {
        rt: 6,
        ra: 8,
        rb: 7,
        oe: false,
        rc: false,
    }
}

#[test]
fn a_fused_or_add_issues_an_or_then_an_add() {
    let or = I::Or {
        ra: 3,
        rs: 4,
        rb: 5,
        rc: false,
    };
    assert_eq!(fused(4, 5), vec![one_op(&or), one_op(&add())]);
}

#[test]
fn a_fused_mr_add_issues_an_mr_then_an_add() {
    let mr = I::Mr { ra: 3, rs: 4 };
    assert_eq!(fused(4, 4), vec![one_op(&mr), one_op(&add())]);
}
