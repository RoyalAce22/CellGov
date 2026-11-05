//! An adjacent `std` off r1 and `lwz` on the fused path, held to the
//! `ppu_std_lwz_pair` capture from the reference console.

use crate::*;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use crate::instruction::PpuInstruction;

/// The probe's `buf` and `alt` addresses here.
const BUF: u64 = 0x800;
const ALT: u64 = 0x900;

/// The words the probe puts in `buf` before each case.
const BUF_WORDS: [u32; 4] = [0xA1A2_A3A4, 0xB1B2_B3B4, 0xC1C2_C3C4, 0xD1D2_D3D4];

/// The doubleword the probe puts in r5.
const STORED: u64 = 0x89AB_CDEF_8000_0001;

/// The value the probe puts in each destination before a case.
const ALL_ONES: u64 = u64::MAX;

/// The probe's payload, as big-endian u32 words.
fn console_words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_std_lwz_pair/ps3/cech20-cex-493/cgov_frame.bin");
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

/// `std rS, ds(rA)`.
const fn std(rs: u32, ra: u32, ds: i16) -> u32 {
    (62 << 26) | (rs << 21) | (ra << 16) | (ds as u16 as u32)
}

/// `lwz rT, d(rA)`.
const fn lwz(rt: u32, ra: u32, d: i16) -> u32 {
    (32 << 26) | (rt << 21) | (ra << 16) | (d as u16 as u32)
}

const B_OVER_ONE: u32 = (18 << 26) | 8;

/// What a case left: r5, r6, and buf words 2 and 3.
struct After {
    r5: u64,
    r6: u64,
    buf_2_3: [u32; 2],
}

/// Runs `words` from address 0 for `steps` instructions over the probe's data.
///
/// The run starts with r1 = `r1`, r4 = [`ALT`], r5 = [`STORED`] and
/// r6 = [`ALL_ONES`]. Asserts that the slot at `second` holds the second
/// half of a fused pair.
fn run(words: &[u32], second: u64, steps: u64, r1: u64) -> After {
    let code = words
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    let shadow = shadow::PredecodedShadow::build(0, &code);
    assert_eq!(
        shadow.get(second),
        Some(PpuInstruction::Consumed),
        "precondition: the pair fuses into 0x{second:x}"
    );
    let mut mem = GuestMemory::new(0x1000);
    let mut put = |addr: u64, words: &[u32]| {
        let bytes = words
            .iter()
            .flat_map(|w| w.to_be_bytes())
            .collect::<Vec<_>>();
        mem.apply_commit(
            ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).unwrap(),
            &bytes,
        )
        .unwrap();
    };
    put(BUF, &BUF_WORDS);
    put(ALT, &[0x1111_2222, 0xA5A5_5A5A]);
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(0), code.len() as u64).unwrap(),
        &code,
    )
    .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);
    unit.state_mut().set_gpr(1, r1);
    unit.state_mut().set_gpr(4, ALT);
    unit.state_mut().set_gpr(5, STORED);
    unit.state_mut().set_gpr(6, ALL_ONES);
    let mut effects = Vec::new();
    unit.run_until_yield(
        Budget::new(steps),
        &ExecutionContext::new(&mem),
        &mut effects,
    );
    assert_eq!(
        unit.retired_instructions(),
        steps,
        "the case ran to its end"
    );
    let mut buf = BUF_WORDS
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    for effect in &effects {
        if let Effect::SharedWriteIntent { range, bytes, .. } = effect {
            let at = usize::try_from(range.start().raw() - BUF).expect("a write into buf");
            buf[at..at + bytes.len()].copy_from_slice(bytes.bytes());
        }
    }
    let word = |n: usize| u32::from_be_bytes(buf[4 * n..4 * n + 4].try_into().expect("4 bytes"));
    let g = &unit.state().gpr;
    After {
        r5: g[5],
        r6: g[6],
        buf_2_3: [word(2), word(3)],
    }
}

#[test]
fn every_std_lwz_pair_case_leaves_what_the_console_left() {
    let w = console_words();
    assert_eq!(w.len(), 20, "payload words");
    assert_eq!(w[0], 0, "the probe ran to its end");

    let after = run(&[std(5, 1, 8), lwz(6, 1, 8)], 4, 2, BUF);
    assert_eq!(
        after.r6,
        console_reg(&w, 2),
        "a load of the stored high word"
    );
    assert_eq!(
        after.buf_2_3,
        [w[18], w[19]],
        "the doubleword the store wrote"
    );

    let after = run(&[std(5, 1, 8), lwz(6, 1, 12)], 4, 2, BUF);
    assert_eq!(
        after.r6,
        console_reg(&w, 4),
        "a load of the stored low word"
    );

    let after = run(&[std(5, 1, 8), lwz(5, 1, 8)], 4, 2, BUF);
    assert_eq!(
        after.r5,
        console_reg(&w, 6),
        "a load into the stored register"
    );

    let after = run(&[std(5, 1, 8), lwz(6, 4, 4)], 4, 2, BUF);
    assert_eq!(after.r6, console_reg(&w, 8), "a load from another base");
    assert_eq!(
        after.buf_2_3,
        [w[10], w[11]],
        "the store beside a load from another base"
    );

    let after = run(&[std(5, 1, -8), lwz(6, 1, -4)], 4, 2, BUF + 16);
    assert_eq!(after.r6, console_reg(&w, 12), "negative displacements");

    let after = run(&[B_OVER_ONE, std(5, 1, 8), lwz(6, 1, 8)], 8, 2, BUF);
    assert_eq!(after.r6, console_reg(&w, 14), "a branch into the load");
    assert_eq!(
        after.buf_2_3,
        [w[16], w[17]],
        "a branch over the store leaves memory as it was"
    );
}
