//! An adjacent `or` and `rlwinm` on the fused path, held to the
//! `ppu_or_rlwinm_pair` capture from the reference console.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use crate::instruction::{FusedPair, PpuInstruction};
use crate::state::PpuState;

/// The probe's payload, as big-endian u32 words.
fn console_words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_or_rlwinm_pair/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
}

/// The 64-bit value the console recorded at payload word `at`.
fn console_reg(w: &[u32], at: usize) -> u64 {
    (u64::from(w[at]) << 32) | u64::from(w[at + 1])
}

/// `or rA, rS, rB`; `mr rA, rS` when `rs == rb`.
const fn or(ra: u32, rs: u32, rb: u32) -> u32 {
    (31 << 26) | (rs << 21) | (ra << 16) | (rb << 11) | (444 << 1)
}

/// `rlwinm rA, rS, sh, mb, me`, with Rc.
const fn rlwinm(ra: u32, rs: u32, sh: u32, mb: u32, me: u32, rc: bool) -> u32 {
    (21 << 26) | (rs << 21) | (ra << 16) | (sh << 11) | (mb << 6) | (me << 1) | rc as u32
}

const B_OVER_ONE: u32 = (18 << 26) | 8;

/// Runs `words` from address 0 for two instructions with CR 0, XER `xer`
/// and the GPRs `regs`.
///
/// # Panics
///
/// Panics if:
/// - the slot at `first` does not hold a fused `or; rlwinm`
/// - the run retires fewer than two instructions
/// - the run does not stop at the end of `words`
fn run(words: &[u32], first: u64, xer: u64, regs: &[(usize, u64)]) -> PpuState {
    let code = words
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    let shadow = shadow::PredecodedShadow::build(0, &code);
    assert!(
        matches!(
            shadow.get(first),
            Some(PpuInstruction::Fused(FusedPair::OrRlwinm { .. }))
        ),
        "precondition: the or and the rlwinm fuse at 0x{first:x}"
    );
    let mut mem = GuestMemory::new(0x1000);
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(0), code.len() as u64).unwrap(),
        &code,
    )
    .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);
    unit.state_mut().set_cr(0);
    unit.state_mut().set_xer(xer);
    for &(reg, value) in regs {
        unit.state_mut().set_gpr(reg, value);
    }
    unit.run_until_yield(
        Budget::new(2),
        &ExecutionContext::new(&mem),
        &mut Vec::new(),
    );
    assert_eq!(unit.retired_instructions(), 2, "the case ran to its end");
    assert_eq!(
        unit.state().pc,
        4 * words.len() as u64,
        "the case left its code"
    );
    unit.state().clone()
}

#[test]
fn every_or_rlwinm_pair_case_leaves_what_the_console_left() {
    let w = console_words();
    assert_eq!(w.len(), 27, "payload words");
    assert_eq!(w[0], 0, "the probe ran to its end");

    let code = [or(5, 6, 7), rlwinm(8, 5, 8, 0, 23, true)];
    let s = run(
        &code,
        0,
        0,
        &[(6, 0x0102_0304_0000_0000), (7, 0x0000_0000_0506_0708)],
    );
    assert_eq!(s.gpr[5], console_reg(&w, 2), "rotate the or result: r5");
    assert_eq!(s.gpr[8], console_reg(&w, 4), "rotate the or result: r8");
    assert_eq!(s.cr(), w[6], "rotate the or result: CR");

    let code = [or(5, 6, 6), rlwinm(8, 5, 8, 24, 7, false)];
    let s = run(&code, 0, 0, &[(6, 0x0102_0304_0506_0708)]);
    assert_eq!(s.gpr[8], console_reg(&w, 7), "mr, wrapping mask: r8");

    let code = [or(5, 6, 7), rlwinm(8, 5, 0, 1, 0, true)];
    let s = run(&code, 0, 0, &[(6, 0x0000_0000_8000_0000), (7, 1)]);
    assert_eq!(s.gpr[8], console_reg(&w, 9), "negative result: r8");
    assert_eq!(s.cr(), w[11], "negative result: CR");

    let code = [or(5, 6, 7), rlwinm(8, 5, 0, 0, 7, true)];
    let s = run(
        &code,
        0,
        0x8000_0000,
        &[(6, 0xABCD_EF00_0000_0000), (7, 0x0000_0000_00FF_FFFF)],
    );
    assert_eq!(s.gpr[8], console_reg(&w, 12), "zero result with SO: r8");
    assert_eq!(s.cr(), w[14], "zero result with SO: CR");

    let code = [or(5, 6, 6), rlwinm(8, 5, 3, 0, 28, false)];
    let s = run(&code, 0, 0, &[(6, 0xFEDC_BA98_7654_3210)]);
    assert_eq!(s.gpr[8], console_reg(&w, 15), "mr, slwi: r8");

    let code = [or(5, 6, 7), rlwinm(5, 5, 16, 0, 31, false)];
    let s = run(
        &code,
        0,
        0,
        &[(6, 0x0123_4567_0000_0000), (7, 0x0000_0000_89AB_CDEF)],
    );
    assert_eq!(
        s.gpr[5],
        console_reg(&w, 17),
        "rotate into the or target: r5"
    );

    let code = [or(5, 6, 7), rlwinm(8, 9, 4, 0, 27, false)];
    let s = run(
        &code,
        0,
        0,
        &[(6, 0x10), (7, 0x20), (9, 0x1122_3344_5566_7788)],
    );
    assert_eq!(s.gpr[5], console_reg(&w, 19), "rotate another register: r5");
    assert_eq!(s.gpr[8], console_reg(&w, 21), "rotate another register: r8");

    let code = [B_OVER_ONE, or(5, 6, 7), rlwinm(8, 5, 4, 0, 27, false)];
    let s = run(
        &code,
        4,
        0,
        &[(5, 0xDEAD_BEEF_DEAD_BEEF), (6, 0x10), (7, 0x20), (8, 0x88)],
    );
    assert_eq!(
        s.gpr[5],
        console_reg(&w, 23),
        "a branch into the rotate: r5"
    );
    assert_eq!(
        s.gpr[8],
        console_reg(&w, 25),
        "a branch into the rotate: r8"
    );
}
