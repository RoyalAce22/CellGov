//! Two adjacent `ld` loads on the fused path, held to the
//! `ppu_ld_ld_pair` capture from the reference console.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use crate::instruction::PpuInstruction;

/// The probe's `buf`, `alt` and `ptrbuf` addresses here.
const BUF: u64 = 0x800;
const ALT: u64 = 0x900;
const PTRBUF: u64 = 0xA00;

/// The probe's payload, as big-endian u32 words.
fn console_words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_ld_ld_pair/ps3/cech20-cex-493/cgov_frame.bin");
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

/// `ld rT, d(rA)`.
const fn ld(rt: u32, ra: u32, d: i16) -> u32 {
    (58 << 26) | (rt << 21) | (ra << 16) | (d as u16 as u32)
}

const B_OVER_ONE: u32 = (18 << 26) | 8;

/// GPRs 4..=6 after `words` run from address 0 for `steps` instructions.
///
/// The run uses the probe's data and starts with r4 = `r4`, r5 = `r5` and
/// r6 = 0. The helper asserts that the slot at `second` holds the second
/// half of a fused pair exactly when `fused` is true.
fn run(words: &[u32], second: u64, fused: bool, steps: u64, r4: u64, r5: u64) -> [u64; 3] {
    let code = words
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    let shadow = shadow::PredecodedShadow::build(0, &code);
    assert_eq!(
        shadow.get(second) == Some(PpuInstruction::Consumed),
        fused,
        "precondition: the slot at 0x{second:x} is fused = {fused}"
    );
    let mut mem = GuestMemory::new(0x1000);
    let mut put = |addr: u64, dwords: &[u64]| {
        let bytes = dwords
            .iter()
            .flat_map(|d| d.to_be_bytes())
            .collect::<Vec<_>>();
        mem.apply_commit(
            ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).unwrap(),
            &bytes,
        )
        .unwrap();
    };
    put(
        BUF,
        &[
            0x0123_4567_89AB_CDEF,
            0x8000_0000_0000_0001,
            0xFEDC_BA98_7654_3210,
            0x7FFF_FFFF_FFFF_FFFE,
        ],
    );
    put(ALT, &[0x1111_1111_2222_2222, 0xA5A5_A5A5_5A5A_5A5A]);
    put(PTRBUF, &[ALT, 0xDEAD_BEEF_CAFE_F00D]);
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(0), code.len() as u64).unwrap(),
        &code,
    )
    .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);
    unit.state_mut().set_gpr(4, r4);
    unit.state_mut().set_gpr(5, r5);
    unit.state_mut().set_gpr(6, 0);
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
    [g[4], g[5], g[6]]
}

#[test]
fn every_ld_pair_case_leaves_the_registers_the_console_left() {
    let w = console_words();
    assert_eq!(w.len(), 18, "payload words");
    assert_eq!(w[0], 0, "the probe ran to its end");

    let [_, r5, r6] = run(&[ld(5, 4, 0), ld(6, 4, 8)], 4, true, 2, BUF, 0);
    assert_eq!(r5, console_reg(&w, 2), "plain pair, first load");
    assert_eq!(r6, console_reg(&w, 4), "plain pair, second load");

    let [_, r5, _] = run(&[ld(5, 4, 8), ld(5, 4, 16)], 4, true, 2, BUF, 0);
    assert_eq!(r5, console_reg(&w, 6), "one register, the second value");

    let code = [B_OVER_ONE, ld(5, 4, 0), ld(6, 4, 8)];
    let [_, r5, r6] = run(&code, 8, true, 2, BUF, 0x0000_0001_0000_0100);
    assert_eq!(r5, console_reg(&w, 8), "a branch into the second load");
    assert_eq!(r6, console_reg(&w, 10), "the second load alone");

    let [_, _, r6] = run(&[ld(4, 4, 0), ld(6, 4, 8)], 4, false, 2, PTRBUF, 0);
    assert_eq!(
        r6,
        console_reg(&w, 12),
        "the second load reads the new base"
    );

    let [_, r5, r6] = run(&[ld(5, 4, -16), ld(6, 4, -8)], 4, true, 2, BUF + 24, 0);
    assert_eq!(r5, console_reg(&w, 14), "negative pair, first load");
    assert_eq!(r6, console_reg(&w, 16), "negative pair, second load");
}
