//! A barrier after a store to a line the L2 does not hold.

use super::super::pressure::stretch_pending;
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;
use crate::timing::{classify, PpeTiming};
use cellgov_ps3_abi::hw::ppu_issue as k;

/// The line every store writes. A fresh model's L2 does not hold it.
const LINE: u64 = 0x100_0000;

const STW: I = I::Stw {
    rs: 10,
    ra: 11,
    imm: 0,
};
const SYNC: I = I::Sync { l: 0 };
const LWSYNC: I = I::Sync { l: 1 };
const ADDI: I = I::Addi {
    rt: 3,
    ra: 4,
    imm: 1,
};

fn state() -> PpuState {
    let mut state = PpuState::new();
    state.set_gpr(11, LINE);
    state
}

/// Retires `insns` once as straight-line code into `timing`, and returns
/// the issue cycle of each op.
fn retire(timing: &mut PpeTiming, insns: &[I]) -> Vec<u64> {
    let state = state();
    let mut cycles = Vec::new();
    for (i, insn) in insns.iter().enumerate() {
        let at = 0x1000 + 4 * i as u64;
        let (ops, n) = classify(insn, &state);
        let issued = timing.retire_instruction(at, &ops[..n], at + 4 * n.max(1) as u64);
        cycles.extend_from_slice(&issued[..n]);
    }
    cycles
}

fn issued(insns: &[I]) -> Vec<u64> {
    retire(&mut PpeTiming::new(), insns)
}

#[test]
fn a_sync_after_a_store_miss_holds_from_the_cycle_the_l2_has_the_line() {
    let cycles = issued(&[STW, SYNC, ADDI]);
    assert_eq!(cycles.len(), 3);
    assert_eq!(
        cycles[2],
        cycles[0] + k::PPU_STORE_MISS_FETCH_CYCLES + k::PPU_SYNC_CYCLES + k::PPU_SYNC_STORE_CYCLES
    );
}

#[test]
fn an_lwsync_after_a_store_miss_does_not_wait_for_the_line() {
    let cycles = issued(&[STW, LWSYNC, ADDI]);
    assert_eq!(cycles.len(), 3);
    assert!(
        cycles[2] < cycles[0] + k::PPU_STORE_MISS_FETCH_CYCLES,
        "the instruction after the lwsync issues {} cycles after the store",
        cycles[2] - cycles[0],
    );
}

#[test]
fn an_isync_after_a_store_miss_does_not_wait_for_the_line() {
    let cycles = issued(&[STW, I::Isync, ADDI]);
    assert_eq!(cycles.len(), 3);
    assert!(
        cycles[2] < cycles[0] + k::PPU_STORE_MISS_FETCH_CYCLES,
        "the instruction after the isync issues {} cycles after the store",
        cycles[2] - cycles[0],
    );
}

#[test]
fn a_sync_after_a_store_to_a_line_the_l2_holds_waits_for_no_fetch() {
    let cycles = issued(&[STW, SYNC, STW, SYNC, ADDI]);
    assert_eq!(cycles.len(), 5);
    assert_eq!(
        cycles[4] - cycles[3],
        k::PPU_SYNC_CYCLES + k::PPU_SYNC_STORE_CYCLES
    );
}

#[test]
fn a_siblings_stall_delays_the_cycle_the_l2_has_a_store_missed_line() {
    let mut timing = PpeTiming::new();
    let cycles = retire(&mut timing, &[STW]);
    let fetched = cycles[0] + k::PPU_STORE_MISS_FETCH_CYCLES;
    assert_eq!(timing.tables.line_fetched, fetched);
    stretch_pending(&mut timing.core, &mut timing.tables, cycles[0], 100);
    assert_eq!(timing.tables.line_fetched, fetched + 100);
}
