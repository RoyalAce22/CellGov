//! An adjacent `cmpd; nor` on the fused path, held to the
//! `ppu_cmpd_nor_pair` capture from the reference console.

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
        .join("../../tests/micro/ppu_cmpd_nor_pair/ps3/cech20-cex-493/cgov_frame.bin");
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

/// `cmpd crF, rA, rB`.
const fn cmpd(bf: u32, ra: u32, rb: u32) -> u32 {
    (31 << 26) | (bf << 23) | (1 << 21) | (ra << 16) | (rb << 11)
}

/// `nor rA, rS, rB` with Rc = 0.
const fn nor(ra: u32, rs: u32, rb: u32) -> u32 {
    (31 << 26) | (rs << 21) | (ra << 16) | (rb << 11) | (124 << 1)
}

const B_OVER_ONE: u32 = (18 << 26) | 8;

/// Runs `words` from address 0 for `steps` instructions with CR 0, XER
/// `xer` and the GPRs `regs`.
///
/// # Panics
///
/// Panics if:
/// - the slot at `first` does not hold a fused `cmpd; nor`
/// - the run retires fewer than `steps` instructions
/// - the run does not stop at the end of `words`
fn run(words: &[u32], first: u64, steps: u64, xer: u64, regs: &[(usize, u64)]) -> PpuState {
    let code = words
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    let shadow = shadow::PredecodedShadow::build(0, &code);
    assert!(
        matches!(
            shadow.get(first),
            Some(PpuInstruction::Fused(FusedPair::CmpdNor { .. }))
        ),
        "precondition: the cmpd and the nor fuse at 0x{first:x}"
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
fn every_cmpd_nor_pair_case_leaves_what_the_console_left() {
    let w = console_words();
    assert_eq!(w.len(), 17, "payload words");
    assert_eq!(w[0], 0, "the probe ran to its end");

    let regs = [(3, 0x1_0000_0000), (4, 1), (6, 0xF0)];
    let s = run(&[cmpd(7, 3, 4), nor(5, 6, 6)], 0, 2, 0, &regs);
    assert_eq!(s.cr(), w[2], "64-bit greater: CR");
    assert_eq!(s.gpr[5], console_reg(&w, 3), "64-bit greater: r5");

    let regs = [
        (3, u64::MAX),
        (4, 1),
        (6, 0xF0F0_F0F0_F0F0_F0F0),
        (8, 0x0F00_0000_0000_00FF),
    ];
    let s = run(&[cmpd(0, 3, 4), nor(5, 6, 8)], 0, 2, 0, &regs);
    assert_eq!(s.cr(), w[5], "less on cr0: CR");
    assert_eq!(s.gpr[5], console_reg(&w, 6), "less on cr0: r5");

    let regs = [(3, 1 << 63), (4, 1 << 63), (6, 0xFF)];
    let s = run(&[cmpd(7, 3, 4), nor(3, 6, 6)], 0, 2, 0, &regs);
    assert_eq!(s.cr(), w[8], "nor target is the first source: CR");
    assert_eq!(
        s.gpr[3],
        console_reg(&w, 9),
        "nor target is the first source: r3"
    );

    let regs = [(3, 1), (4, 2), (6, 0xF0)];
    let s = run(&[cmpd(7, 3, 4), nor(5, 6, 6)], 0, 2, 0x8000_0000, &regs);
    assert_eq!(s.cr(), w[11], "SO set: CR");
    assert_eq!(s.gpr[5], console_reg(&w, 12), "SO set: r5");

    let regs = [(3, 1), (4, 1), (6, 0xF0)];
    let s = run(&[B_OVER_ONE, cmpd(7, 3, 4), nor(5, 6, 6)], 4, 2, 0, &regs);
    assert_eq!(s.cr(), w[14], "a branch over the cmpd: CR");
    assert_eq!(s.gpr[5], console_reg(&w, 15), "a branch over the cmpd: r5");
}
