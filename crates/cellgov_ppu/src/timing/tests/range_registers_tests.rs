//! The registers a load string, a store multiple and an overflow-enabled
//! multiply read or write past the ones they name first.

use super::op::XER;
use super::{classify, Op, PpeTiming};
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;

/// The single op `insn` classifies to.
fn one_op(insn: &I) -> Op {
    let (ops, n) = classify(insn, &PpuState::new());
    assert_eq!(n, 1);
    ops[0]
}

/// The issue cycle of `stmw r24,0(r4)` after `ld rt,0(r3)` misses the
/// caches.
fn stmw_after_a_missing_load_into(rt: u8) -> u64 {
    let mut timing = PpeTiming::new();
    let mut state = PpuState::new();
    state.set_gpr(3, 0x10_0000);
    state.set_gpr(4, 0x20_0000);
    let insns = [
        I::Ld { rt, ra: 3, imm: 0 },
        I::Stmw {
            rs: 24,
            ra: 4,
            imm: 0,
        },
    ];
    let mut cycles = Vec::new();
    for (i, insn) in insns.iter().enumerate() {
        let at = 0x1000 + 4 * i as u64;
        let (ops, n) = classify(insn, &state);
        cycles.extend_from_slice(&timing.retire_instruction(at, &ops[..n], at + 4)[..n]);
    }
    cycles[1]
}

#[test]
fn an_lswi_of_eight_bytes_into_r31_writes_r0() {
    let op = one_op(&I::Lswi {
        rt: 31,
        ra: 10,
        nb: 8,
    });
    let writes: Vec<u8> = op.writes().collect();
    assert_eq!(writes, [31, 0]);
}

#[test]
fn a_stmw_waits_for_its_last_source() {
    let last = stmw_after_a_missing_load_into(31);
    let outside = stmw_after_a_missing_load_into(23);
    assert!(
        last > outside,
        "stmw r24 issues at {last} after a missing load into r31, at {outside} after one \
         into r23"
    );
}

#[test]
fn mullwo_and_mulldo_write_xer_and_mullw_and_mulld_do_not() {
    let mullw = |oe| I::Mullw {
        rt: 3,
        ra: 4,
        rb: 5,
        oe,
        rc: false,
    };
    let mulld = |oe| I::Mulld {
        rt: 3,
        ra: 4,
        rb: 5,
        oe,
        rc: false,
    };
    for (name, insn, writes_xer) in [
        ("mullwo", mullw(true), true),
        ("mulldo", mulld(true), true),
        ("mullw", mullw(false), false),
        ("mulld", mulld(false), false),
    ] {
        assert_eq!(
            one_op(&insn).writes().any(|r| r == XER),
            writes_xer,
            "{name}"
        );
    }
}
