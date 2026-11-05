//! Two adjacent `ldu` loads on the fused path, held to the
//! `ppu_ldu_ldu_pair` capture from the reference console.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use crate::instruction::PpuInstruction;
use crate::state::PpuState;

/// The probe's `data` address here.
const DATA: u64 = 0x800;

/// The probe's payload, as big-endian u32 words.
fn console_words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_ldu_ldu_pair/ps3/cech20-cex-493/cgov_frame.bin");
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

/// `ldu rT, d(rA)`.
const fn ldu(rt: u32, ra: u32, d: i16) -> u32 {
    (58 << 26) | (rt << 21) | (ra << 16) | ((d as u16 as u32) & 0xFFFC) | 1
}

const B_OVER_ONE: u32 = (18 << 26) | 8;

/// Runs `words` from address 0 for `steps` instructions and returns the state.
///
/// The run starts from the GPRs `regs` over the probe's data.
///
/// # Panics
///
/// Panics if:
/// - the slot at `second` does not hold the second half of a fused pair
/// - the run retires fewer than `steps` instructions
/// - the run does not stop at the end of `words`
fn run(words: &[u32], second: u64, steps: u64, regs: &[(usize, u64)]) -> PpuState {
    let code = words
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    let shadow = shadow::PredecodedShadow::build(0, &code);
    assert_eq!(
        shadow.get(second),
        Some(PpuInstruction::Consumed),
        "precondition: the pair before 0x{second:x} fuses"
    );
    let data = [
        0x0123_4567_89AB_CDEF_u64,
        0xFEDC_BA98_7654_3210,
        0x8000_0000_0000_0001,
        0x7FFF_FFFF_FFFF_FFFE,
    ]
    .iter()
    .flat_map(|d| d.to_be_bytes())
    .collect::<Vec<_>>();
    let mut mem = GuestMemory::new(0x1000);
    for (addr, bytes) in [(0, code.as_slice()), (DATA, data.as_slice())] {
        mem.apply_commit(
            ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).unwrap(),
            bytes,
        )
        .unwrap();
    }
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);
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
fn every_ldu_pair_case_leaves_what_the_console_left() {
    let w = console_words();
    assert_eq!(w.len(), 42, "payload words");
    assert_eq!(w[0], 0, "the probe ran to its end");
    let a0 = DATA - 8;
    let b0 = DATA + 8;
    let n0 = DATA + 32;

    let s = run(&[ldu(7, 5, 8), ldu(8, 5, 8)], 4, 2, &[(5, a0)]);
    assert_eq!(s.gpr[7], console_reg(&w, 2), "one base: r7");
    assert_eq!(s.gpr[8], console_reg(&w, 4), "one base: r8");
    assert_eq!(s.gpr[5] - a0, console_reg(&w, 6), "one base: r5 distance");

    let regs = [(5, a0), (6, b0)];
    let s = run(&[ldu(7, 5, 8), ldu(8, 6, 8)], 4, 2, &regs);
    assert_eq!(s.gpr[7], console_reg(&w, 8), "two bases: r7");
    assert_eq!(s.gpr[8], console_reg(&w, 10), "two bases: r8");
    assert_eq!(s.gpr[5] - a0, console_reg(&w, 12), "two bases: r5 distance");
    assert_eq!(s.gpr[6] - b0, console_reg(&w, 14), "two bases: r6 distance");

    let s = run(&[ldu(7, 5, 8), ldu(5, 6, 8)], 4, 2, &regs);
    assert_eq!(s.gpr[7], console_reg(&w, 16), "second target r5: r7");
    assert_eq!(s.gpr[5], console_reg(&w, 18), "second target r5: r5");
    assert_eq!(
        s.gpr[6] - b0,
        console_reg(&w, 20),
        "second target r5: r6 distance"
    );

    let s = run(&[ldu(7, 5, 8), ldu(7, 6, 8)], 4, 2, &regs);
    assert_eq!(s.gpr[7], console_reg(&w, 22), "one target: r7");
    assert_eq!(
        s.gpr[5] - a0,
        console_reg(&w, 24),
        "one target: r5 distance"
    );
    assert_eq!(
        s.gpr[6] - b0,
        console_reg(&w, 26),
        "one target: r6 distance"
    );

    let code = [B_OVER_ONE, ldu(7, 5, 8), ldu(8, 6, 8)];
    let s = run(&code, 8, 2, &[(5, a0), (6, b0), (7, 0x77)]);
    assert_eq!(s.gpr[7], console_reg(&w, 28), "a branch over the first: r7");
    assert_eq!(s.gpr[8], console_reg(&w, 30), "a branch over the first: r8");
    assert_eq!(
        s.gpr[5] - a0,
        console_reg(&w, 32),
        "a branch over the first: r5 distance"
    );
    assert_eq!(
        s.gpr[6] - b0,
        console_reg(&w, 34),
        "a branch over the first: r6 distance"
    );

    let s = run(&[ldu(7, 5, -8), ldu(8, 5, -8)], 4, 2, &[(5, n0)]);
    assert_eq!(s.gpr[7], console_reg(&w, 36), "negative offsets: r7");
    assert_eq!(s.gpr[8], console_reg(&w, 38), "negative offsets: r8");
    assert_eq!(
        s.gpr[5].wrapping_sub(n0),
        console_reg(&w, 40),
        "negative offsets: r5 distance"
    );
}
