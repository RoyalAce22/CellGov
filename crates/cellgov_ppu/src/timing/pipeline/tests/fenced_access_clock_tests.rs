//! An FP load or store that an `mtvscr` holds in the VSU issue queue,
//! and the cycle its cache access takes.
//!
//! [CBE-Handbook p:757 s:A.6.2] A VXU or FPU load goes to the LSU for the L1 at issue, and its data waits in a load target buffer until the VSU issue queue issues the load.
//!
//! [CBE-Handbook p:756 s:A.6.2] Figure A-3 sends the LSU part of a VXU or FPU store from IU issue, beside the VSU issue queue.
//!
//! The `ppu_issue_vscr_miss` capture shows that an `mtvscr` before an
//! `lfd` adds its whole hold to the load's data, both when the load
//! misses the L2 and when it hits the L1.

use crate::instruction::ops::Fp63Op;
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;
use crate::timing::{classify, PpeTiming};
use cellgov_ps3_abi::hw::ppu_issue as k;

/// The bytes between two lines of one L1 set.
const SET_STRIDE: u64 = k::PPU_L1D_SETS as u64 * k::PPU_CACHE_LINE_BYTES;

/// Lines A to E of one L1 set, in r11 to r15. A to D fill the set's
/// ways; E's reload then takes A's way.
const SET_LINES: [u64; 5] = [
    0x100_0000,
    0x100_0000 + SET_STRIDE,
    0x100_0000 + 2 * SET_STRIDE,
    0x100_0000 + 3 * SET_STRIDE,
    0x100_0000 + 4 * SET_STRIDE,
];
/// A line of another L1 set, in r16: the address of the fenced access.
const OTHER_LINE: u64 = 0x100_0000 + k::PPU_CACHE_LINE_BYTES;

const fn lwz(ra: u8) -> I {
    I::Lwz { rt: 3, ra, imm: 0 }
}
/// `addi r4,r3,1`, which waits for the last [`lwz`].
const USE_R3: I = I::Addi {
    rt: 4,
    ra: 3,
    imm: 1,
};
const MTVSCR: I = I::Mtvscr { vb: 0 };
const LFD: I = I::Lfd {
    frt: 1,
    ra: 16,
    imm: 0,
};
const STFD: I = I::Stfd {
    frs: 2,
    ra: 16,
    imm: 0,
};
/// `fadd f3,f1,f1`, which waits for [`LFD`].
const USE_F1: I = I::Fp63 {
    op: Fp63Op::Fadd,
    frt: 3,
    fra: 1,
    frb: 1,
    frc: 0,
    rc: false,
};
/// `dcbt 0,r11`: a touch of line A.
const TOUCH_A: I = I::Dcbt {
    ra: 0,
    rb: 11,
    th: 0,
};

/// `lwz r5,0(r15)`: a load of line E.
const LOAD_E: I = I::Lwz {
    rt: 5,
    ra: 15,
    imm: 0,
};

/// The instructions that put line E in the L2 alone and lines A to D in
/// the L1:
/// - `dcbz` puts E in the L2 alone;
/// - each of A to D is loaded and its result used, so its reload writes.
///
/// A [`LOAD_E`] after them hits the L2, and its reload takes A's way.
fn set_filled() -> Vec<I> {
    let mut insns = vec![I::Dcbz { ra: 0, rb: 15 }];
    for ra in 11..=14 {
        insns.extend([lwz(ra), USE_R3]);
    }
    insns
}

/// Retires `insns` once as straight-line code into `timing`, and returns
/// the issue cycle of each instruction's first op.
fn retire(timing: &mut PpeTiming, insns: &[I]) -> Vec<u64> {
    let mut state = PpuState::new();
    for (reg, line) in (11..).zip(SET_LINES) {
        state.set_gpr(reg, line);
    }
    state.set_gpr(16, OTHER_LINE);
    let mut cycles = Vec::new();
    for (i, insn) in insns.iter().enumerate() {
        let at = 0x1000 + 4 * i as u64;
        let (ops, n) = classify(insn, &state);
        let issued = timing.retire_instruction(at, &ops[..n], at + 4 * n.max(1) as u64);
        cycles.push(issued[0]);
    }
    cycles
}

/// Retires [`set_filled`], an `mtvscr`, [`LOAD_E`], `access` and a touch
/// of A. The `mtvscr` waits for older results, so it comes before the
/// load of E.
///
/// Returns whether the touch sent a request. Before it returns, it checks
/// these:
/// - the touch issues before the reload of E can return from the L2;
/// - in the end, the L1 holds B to E, so the touch brought no copy of A.
fn touch_sends_request_after_fenced(access: I) -> bool {
    let mut insns = set_filled();
    insns.extend([MTVSCR, LOAD_E, access, TOUCH_A]);
    let mut timing = PpeTiming::new();
    let cycles = retire(&mut timing, &insns);
    assert_eq!(cycles.len(), insns.len());
    let gap = cycles[insns.len() - 1] - cycles[insns.len() - 3];
    assert!(
        gap < k::PPU_L2_LOAD_USE_CYCLES,
        "the touch issues {gap} cycles after the load of E, after its reload"
    );
    let held: Vec<bool> = SET_LINES
        .iter()
        .map(|line| timing.tables.caches.holds(*line, *line).0)
        .collect();
    assert_eq!(held, [false, true, true, true, true]);
    timing.tables.caches.touch_free() != 0
}

#[test]
fn a_touch_after_a_fenced_fp_load_finds_the_line_that_a_later_reload_evicts() {
    assert!(!touch_sends_request_after_fenced(LFD));
}

#[test]
fn a_touch_after_a_fenced_fp_store_finds_the_line_that_a_later_reload_evicts() {
    assert!(!touch_sends_request_after_fenced(STFD));
}

#[test]
fn a_fenced_fp_load_that_misses_gives_its_data_a_memory_trip_after_the_vsu_issue_queue_lets_it_go()
{
    let insns = [MTVSCR, LFD, USE_F1];
    let cycles = retire(&mut PpeTiming::new(), &insns);
    assert_eq!(cycles.len(), insns.len());
    assert!(
        cycles[2] >= cycles[0] + k::PPU_MTVSCR_YOUNGER_VSU_CYCLES + k::PPU_MEMORY_LOAD_USE_CYCLES,
        "the use issues {} cycles after the mtvscr",
        cycles[2] - cycles[0],
    );
}

#[test]
fn a_fenced_fp_load_that_hits_gives_its_data_after_the_vsu_issue_queue_lets_it_go() {
    let insns = [
        I::Lwz {
            rt: 3,
            ra: 16,
            imm: 0,
        },
        USE_R3,
        MTVSCR,
        LFD,
        USE_F1,
    ];
    let cycles = retire(&mut PpeTiming::new(), &insns);
    assert_eq!(cycles.len(), insns.len());
    assert!(
        cycles[4] >= cycles[2] + k::PPU_MTVSCR_YOUNGER_VSU_CYCLES + k::PPU_FP_LOAD_LATENCY_CYCLES,
        "the use issues {} cycles after the mtvscr",
        cycles[4] - cycles[2],
    );
}
