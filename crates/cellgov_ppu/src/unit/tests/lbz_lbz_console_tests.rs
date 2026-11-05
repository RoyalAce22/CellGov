//! Two adjacent `lbz` loads on the fused path, held to the
//! `ppu_lbz_lbz_pair` capture from the reference console.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use crate::instruction::PpuInstruction;

/// The probe's `buf` and `alt` addresses in the test memory.
const BUF: u64 = 0x800;
const ALT: u64 = 0x900;

/// The value the probe puts in each destination before a case.
const ALL_ONES: u64 = u64::MAX;

/// The probe's payload, as big-endian u32 words.
fn console_words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_lbz_lbz_pair/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
}

/// The 64-bit register the console recorded at payload word `at`.
fn console_reg(w: &[u32], at: usize) -> u64 {
    (u64::from(w[at]) << 32) | u64::from(w[at + 1])
}

/// `lbz rT, d(rA)`.
const fn lbz(rt: u32, ra: u32, d: i16) -> u32 {
    (34 << 26) | (rt << 21) | (ra << 16) | (d as u16 as u32)
}

const B_OVER_ONE: u32 = (18 << 26) | 8;

/// GPRs 4..=7 after `words` run from address 0 for `steps` instructions.
///
/// The run uses the probe's data and starts with GPRs 4..=7 = `regs`.
/// The helper asserts that the slot at `second` holds the second half of
/// a fused pair.
fn run(words: &[u32], second: u64, steps: u64, regs: [u64; 4]) -> [u64; 4] {
    let code = words
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    let shadow = shadow::PredecodedShadow::build(0, &code);
    assert_eq!(
        shadow.get(second),
        Some(PpuInstruction::Consumed),
        "precondition: the slot at 0x{second:x} is fused"
    );
    let mut mem = GuestMemory::new(0x1000);
    let mut put = |addr: u64, bytes: &[u8]| {
        mem.apply_commit(
            ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).unwrap(),
            bytes,
        )
        .unwrap();
    };
    put(BUF, &[0x89, 0x80, 0x01, 0x7F, 0xFF, 0x00, 0x5A, 0xA5]);
    put(ALT, &[0x11, 0xC3, 0x22, 0xEE]);
    put(0, &code);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);
    for (n, v) in regs.into_iter().enumerate() {
        unit.state_mut().set_gpr(4 + n, v);
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
    let g = &unit.state().gpr;
    [g[4], g[5], g[6], g[7]]
}

#[test]
fn every_lbz_pair_case_leaves_the_registers_the_console_left() {
    let w = console_words();
    assert_eq!(w.len(), 28, "payload words");
    assert_eq!(w[0], 0, "the probe ran to its end");

    let code = [lbz(5, 4, 0), lbz(6, 4, 1)];
    let [_, r5, r6, _] = run(&code, 4, 2, [BUF, ALL_ONES, ALL_ONES, 0]);
    assert_eq!(r5, console_reg(&w, 2), "plain pair, first load");
    assert_eq!(r6, console_reg(&w, 4), "plain pair, second load");

    let code = [lbz(5, 4, 3), lbz(5, 4, 4)];
    let [_, r5, _, _] = run(&code, 4, 2, [BUF, ALL_ONES, 0, 0]);
    assert_eq!(r5, console_reg(&w, 6), "one register, the second value");

    let code = [B_OVER_ONE, lbz(5, 4, 0), lbz(6, 4, 1)];
    let [_, r5, r6, _] = run(&code, 8, 2, [BUF, 0x0000_0001_0000_0100, ALL_ONES, 0]);
    assert_eq!(r5, console_reg(&w, 8), "a branch into the second load");
    assert_eq!(r6, console_reg(&w, 10), "the second load alone");

    let code = [lbz(4, 4, 4), lbz(6, 7, 1)];
    let [r4, _, r6, _] = run(&code, 4, 2, [BUF, 0, ALL_ONES, ALT]);
    assert_eq!(r4, console_reg(&w, 12), "a first load into its own base");
    assert_eq!(r6, console_reg(&w, 14), "the second load from another base");

    let code = [lbz(5, 4, -8), lbz(6, 7, -3)];
    let [_, r5, r6, _] = run(&code, 4, 2, [BUF + 8, ALL_ONES, ALL_ONES, ALT + 4]);
    assert_eq!(r5, console_reg(&w, 16), "negative pair, first load");
    assert_eq!(r6, console_reg(&w, 18), "negative pair, second load");

    let code = [lbz(5, 4, 6), lbz(7, 7, 3)];
    let [_, r5, _, r7] = run(&code, 4, 2, [BUF, ALL_ONES, 0, ALT]);
    assert_eq!(
        r5,
        console_reg(&w, 20),
        "first load before a self-base second"
    );
    assert_eq!(r7, console_reg(&w, 22), "a second load into its own base");

    let code = [lbz(5, 4, 7), lbz(4, 7, 2)];
    let [r4, r5, _, _] = run(&code, 4, 2, [BUF, ALL_ONES, 0, ALT]);
    assert_eq!(
        r5,
        console_reg(&w, 24),
        "first load whose base the second overwrites"
    );
    assert_eq!(
        r4,
        console_reg(&w, 26),
        "a second load into the first load's base"
    );
}
