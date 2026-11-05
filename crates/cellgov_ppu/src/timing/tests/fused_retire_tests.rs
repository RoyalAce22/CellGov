//! A fused compare-and-branch retires its branch at `pc + 4`. The branch
//! takes its direction from the CR field its compare writes. It joins
//! the fetch block and the dispatch group of the compare.

use super::{classify, PpeTiming};
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;

const PC: u64 = 0x1000;

/// BO 12 branches when the CR bit is set and carries no static hint, so
/// the branch history table predicts it.
const BO: u8 = 12;

/// CR0 EQ.
const BI: u8 = 2;

const OFFSET: i16 = 0x40;

fn addi() -> I {
    I::Addi {
        rt: 14,
        ra: 14,
        imm: 1,
    }
}

/// Retire `insn` at `pc` with `next_pc` as its next address, and return
/// the issue cycle of each of its ops.
fn retire(timing: &mut PpeTiming, state: &PpuState, pc: u64, insn: &I, next_pc: u64) -> Vec<u64> {
    let (ops, n) = classify(insn, state);
    timing.retire_instruction(pc, &ops[..n], next_pc)[..n].to_vec()
}

/// The address the thread runs after the branch.
fn next_pc(taken: bool) -> u64 {
    if taken {
        PC + 4 + OFFSET as u64
    } else {
        PC + 8
    }
}

/// The issue cycles of a fused `cmpwi r3, 0; bc` at [`PC`], and of an
/// `addi` at the address the thread runs after the pair. When `taken`, r3
/// is zero, so the compare sets EQ and the branch is taken.
fn fused(taken: bool) -> Vec<u64> {
    let mut timing = PpeTiming::new();
    let mut state = PpuState::new();
    state.set_gpr(3, u64::from(!taken));
    let next_pc = next_pc(taken);
    let pair = I::CmpwiBc {
        bf: 0,
        ra: 3,
        imm: 0,
        bo: BO,
        bi: BI,
        target_offset: OFFSET,
    };
    let mut cycles = retire(&mut timing, &state, PC, &pair, next_pc);
    cycles.extend(retire(&mut timing, &state, next_pc, &addi(), next_pc + 4));
    cycles
}

/// The issue cycles of the same `cmpwi` and `bc` as two instructions, and
/// of the `addi` at the branch's next address. The CR gives the branch its
/// direction.
fn unfused(taken: bool) -> Vec<u64> {
    let mut timing = PpeTiming::new();
    let mut state = PpuState::new();
    state.set_cr_field(0, if taken { 0b0010 } else { 0 });
    let next_pc = next_pc(taken);
    let cmpwi = I::Cmpwi {
        bf: 0,
        ra: 3,
        imm: 0,
    };
    let bc = I::Bc {
        bo: BO,
        bi: BI,
        offset: OFFSET,
        aa: false,
        link: false,
    };
    let mut cycles = retire(&mut timing, &state, PC, &cmpwi, PC + 4);
    cycles.extend(retire(&mut timing, &state, PC + 4, &bc, next_pc));
    cycles.extend(retire(&mut timing, &state, next_pc, &addi(), next_pc + 4));
    cycles
}

#[test]
fn a_fused_pair_that_falls_through_times_as_a_compare_and_a_branch_not_taken() {
    let cycles = fused(false);
    assert_eq!(cycles.len(), 3);
    assert_eq!(cycles, unfused(false));
    assert_ne!(cycles, unfused(true));
}

#[test]
fn a_fused_pair_that_branches_times_as_a_compare_and_a_branch_taken() {
    let cycles = fused(true);
    assert_eq!(cycles.len(), 3);
    assert_eq!(cycles, unfused(true));
    assert_ne!(cycles, unfused(false));
}
