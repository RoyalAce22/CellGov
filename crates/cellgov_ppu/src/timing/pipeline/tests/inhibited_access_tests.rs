//! A caching-inhibited load or store, which reads no cache and changes
//! no cache tag.
//!
//! [CBE-Handbook p:134 s:6.1.2.2] The CIU sends an access to the NCU or to the L2 by the page's I bit.
//!
//! [CBE-Handbook p:208 s:8.4.1.1] A caching-inhibited load or store is an EIB operation of the instruction's own length, and only a cacheable L2 miss reads a 128-byte line.

use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;
use crate::timing::{classify, PpeTiming};
use cellgov_ps3_abi::hw::ppu_issue as k;

/// The line every access reaches. A fresh model's caches do not hold it.
const LINE: u64 = 0x100_0000;

const STW: I = I::Stw {
    rs: 10,
    ra: 11,
    imm: 0,
};
const LWZ: I = I::Lwz {
    rt: 3,
    ra: 11,
    imm: 0,
};
const SYNC: I = I::Sync { l: 0 };
/// `addi r5,r3,1`, which uses the result of [`LWZ`].
const USE_R3: I = I::Addi {
    rt: 5,
    ra: 3,
    imm: 1,
};

/// Retires each instruction of `insns` once as straight-line code into
/// `timing`, with r11 at [`LINE`]. Each instruction's load or store is
/// caching-inhibited when its flag is true. Returns the issue cycle of
/// each instruction's first op.
fn retire(timing: &mut PpeTiming, insns: &[(I, bool)]) -> Vec<u64> {
    let mut state = PpuState::new();
    state.set_gpr(11, LINE);
    let mut cycles = Vec::new();
    for (i, &(insn, inhibited)) in insns.iter().enumerate() {
        let at = 0x1000 + 4 * i as u64;
        let (mut ops, n) = classify(&insn, &state);
        for op in &mut ops[..n] {
            op.mark_inhibited(|_, _| inhibited);
        }
        let issued = timing.retire_instruction(at, &ops[..n], at + 4 * n.max(1) as u64);
        cycles.push(issued[0]);
    }
    cycles
}

#[test]
fn a_caching_inhibited_store_leaves_no_line_in_the_caches_and_fetches_none() {
    let mut timing = PpeTiming::new();
    retire(&mut timing, &[(STW, true)]);
    assert_eq!(timing.tables.caches.holds(LINE, LINE), (false, false));
    assert_eq!(timing.tables.line_fetched, 0);
}

#[test]
fn a_sync_after_a_caching_inhibited_store_waits_for_no_line_fetch() {
    let cycles = retire(
        &mut PpeTiming::new(),
        &[(STW, true), (SYNC, false), (USE_R3, false)],
    );
    assert_eq!(cycles.len(), 3);
    assert!(
        cycles[2] < cycles[0] + k::PPU_STORE_MISS_FETCH_CYCLES,
        "the instruction after the sync issues {} cycles after the store",
        cycles[2] - cycles[0],
    );
}

#[test]
fn a_caching_inhibited_load_leaves_no_line_in_the_caches() {
    let mut timing = PpeTiming::new();
    retire(&mut timing, &[(LWZ, true)]);
    assert_eq!(timing.tables.caches.holds(LINE, LINE), (false, false));
}

/// The cycles from the first of two loads of [`LINE`] to the use of its
/// result, and the same for the second load.
fn load_use_gaps(inhibited: bool) -> (u64, u64) {
    let pair = [(LWZ, inhibited), (USE_R3, false)];
    let cycles = retire(&mut PpeTiming::new(), &[pair, pair].concat());
    assert_eq!(cycles.len(), 4);
    (cycles[1] - cycles[0], cycles[3] - cycles[2])
}

#[test]
fn a_second_caching_inhibited_load_of_a_line_takes_the_trip_of_the_first() {
    let (first, second) = load_use_gaps(true);
    let (_, cached_second) = load_use_gaps(false);
    assert_eq!(second, first);
    assert!(
        second > cached_second,
        "a cacheable second load hits the L1: {second} vs {cached_second}"
    );
}
