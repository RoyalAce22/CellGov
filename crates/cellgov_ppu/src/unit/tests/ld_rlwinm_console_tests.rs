//! An adjacent `ld` from r1 and `rlwinm` on the fused path, held to the
//! `ppu_ld_rlwinm_pair` capture from the reference console.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use crate::instruction::{FusedPair, PpuInstruction};
use crate::state::PpuState;

/// The stack pointer here; each case's doubleword sits at `R1 - 8`.
const R1: u64 = 0x800;

/// The probe's payload, as big-endian u32 words.
fn console_words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_ld_rlwinm_pair/ps3/cech20-cex-493/cgov_frame.bin");
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

/// `ld rT, d(r1)`.
const fn ld_r1(rt: u32, d: i16) -> u32 {
    (58 << 26) | (rt << 21) | (1 << 16) | (d as u16 as u32 & !3)
}

/// `rlwinm rA, rS, sh, mb, me`, with Rc.
const fn rlwinm(ra: u32, rs: u32, sh: u32, mb: u32, me: u32, rc: bool) -> u32 {
    (21 << 26) | (rs << 21) | (ra << 16) | (sh << 11) | (mb << 6) | (me << 1) | rc as u32
}

const B_OVER_ONE: u32 = (18 << 26) | 8;

/// Runs `words` from address 0 for `steps` instructions with CR 0, XER
/// `xer`, `v` at `R1 - 8` and the GPRs `regs`.
///
/// # Panics
///
/// Panics if:
/// - the slot at `first` does not hold a fused `ld; rlwinm`
/// - the run retires fewer than `steps` instructions
/// - the run does not stop at the end of `words`
fn run(words: &[u32], first: u64, steps: u64, v: u64, xer: u64, regs: &[(usize, u64)]) -> PpuState {
    let code = words
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    let shadow = shadow::PredecodedShadow::build(0, &code);
    assert!(
        matches!(
            shadow.get(first),
            Some(PpuInstruction::Fused(FusedPair::LdRlwinm { .. }))
        ),
        "precondition: the ld and the rlwinm fuse at 0x{first:x}"
    );
    let mut mem = GuestMemory::new(0x1000);
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(0), code.len() as u64).unwrap(),
        &code,
    )
    .unwrap();
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(R1 - 8), 8).unwrap(),
        &v.to_be_bytes(),
    )
    .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);
    unit.state_mut().set_cr(0);
    unit.state_mut().set_xer(xer);
    unit.state_mut().set_gpr(1, R1);
    for &(reg, value) in regs {
        unit.state_mut().set_gpr(reg, value);
    }
    unit.run_until_yield(
        Budget::new(steps),
        &ExecutionContext::new(&mem),
        &mut Vec::new(),
    );
    assert_eq!(
        unit.retired_instructions(),
        steps,
        "the case ran to its end"
    );
    assert_eq!(
        unit.state().pc,
        4 * words.len() as u64,
        "the case left its code"
    );
    unit.state().clone()
}

#[test]
fn every_ld_rlwinm_pair_case_leaves_what_the_console_left() {
    let w = console_words();
    assert_eq!(w.len(), 31, "payload words");
    assert_eq!(w[0], 0, "the probe ran to its end");

    let code = [ld_r1(6, -8), rlwinm(7, 6, 8, 0, 23, true)];
    let s = run(&code, 0, 2, 0x0102_0304_0506_0708, 0, &[]);
    assert_eq!(s.gpr[6], console_reg(&w, 2), "rotate the loaded value: r6");
    assert_eq!(s.gpr[7], console_reg(&w, 4), "rotate the loaded value: r7");
    assert_eq!(s.cr(), w[6], "rotate the loaded value: CR");

    let code = [ld_r1(6, -8), rlwinm(7, 6, 8, 24, 7, false)];
    let s = run(&code, 0, 2, 0x0102_0304_0506_0708, 0, &[]);
    assert_eq!(s.gpr[7], console_reg(&w, 7), "wrapping mask: r7");

    let code = [ld_r1(6, -8), rlwinm(7, 6, 0, 1, 0, true)];
    let s = run(&code, 0, 2, 0x0000_0000_8000_0001, 0, &[]);
    assert_eq!(s.gpr[7], console_reg(&w, 9), "negative result: r7");
    assert_eq!(s.cr(), w[11], "negative result: CR");

    let code = [ld_r1(6, -8), rlwinm(7, 6, 0, 0, 7, true)];
    let s = run(&code, 0, 2, 0xABCD_EF00_00FF_FFFF, 0x8000_0000, &[]);
    assert_eq!(s.gpr[7], console_reg(&w, 12), "zero result with SO: r7");
    assert_eq!(s.cr(), w[14], "zero result with SO: CR");

    // slwi 3, srwi 3, clrlwi 3.
    for ((sh, mb, me), at, form) in [
        ((3, 0, 28), 15, "slwi"),
        ((29, 3, 31), 17, "srwi"),
        ((0, 3, 31), 19, "clrlwi"),
    ] {
        let code = [ld_r1(6, -8), rlwinm(7, 6, sh, mb, me, false)];
        let s = run(&code, 0, 2, 0xFEDC_BA98_7654_3210, 0, &[]);
        assert_eq!(s.gpr[7], console_reg(&w, at), "{form}: r7");
    }

    let code = [ld_r1(6, -8), rlwinm(6, 6, 16, 0, 31, false)];
    let s = run(&code, 0, 2, 0x0123_4567_89AB_CDEF, 0, &[]);
    assert_eq!(
        s.gpr[6],
        console_reg(&w, 21),
        "rotate into the load target: r6"
    );

    let code = [ld_r1(6, -8), rlwinm(7, 5, 4, 0, 27, false)];
    let s = run(&code, 0, 2, u64::MAX, 0, &[(5, 0x1122_3344_5566_7788)]);
    assert_eq!(s.gpr[6], console_reg(&w, 23), "rotate another register: r6");
    assert_eq!(s.gpr[7], console_reg(&w, 25), "rotate another register: r7");

    let code = [B_OVER_ONE, ld_r1(6, -8), rlwinm(7, 6, 4, 0, 27, false)];
    let s = run(
        &code,
        4,
        2,
        0xDEAD_BEEF_DEAD_BEEF,
        0,
        &[(6, 0x66), (7, 0x77)],
    );
    assert_eq!(
        s.gpr[6],
        console_reg(&w, 27),
        "a branch into the rotate: r6"
    );
    assert_eq!(
        s.gpr[7],
        console_reg(&w, 29),
        "a branch into the rotate: r7"
    );
}
