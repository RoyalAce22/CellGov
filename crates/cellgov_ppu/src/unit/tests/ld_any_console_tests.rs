//! Two adjacent `ld` loads outside the same-base pair, held to the
//! `ppu_ld_any_pair` capture from the reference console.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use crate::instruction::{FusedPair, PpuInstruction};

/// The probe's `buf`, `alt` and `ptrbuf` addresses here.
const BUF: u64 = 0x800;
const ALT: u64 = 0x900;
const PTRBUF: u64 = 0xA00;

/// The value the probe puts in each destination before a case.
const ALL_ONES: u64 = u64::MAX;

/// The probe's payload, as big-endian u32 words.
fn console_words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_ld_any_pair/ps3/cech20-cex-493/cgov_frame.bin");
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

/// GPRs 4..=7 after the two loads in `words` run from address 0.
///
/// The run uses the probe's data and starts with GPRs 4..=7 = `regs`.
///
/// # Panics
///
/// Panics if:
/// - whether the loads fuse as `LdLdAny` differs from `fused`
/// - the two loads do not both retire
fn run(words: [u32; 2], fused: bool, regs: [u64; 4]) -> [u64; 4] {
    let code = words
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    let shadow = shadow::PredecodedShadow::build(0, &code);
    assert_eq!(
        matches!(
            shadow.get(0),
            Some(PpuInstruction::Fused(FusedPair::LdLdAny { .. }))
        ),
        fused,
        "precondition: the loads fuse as LdLdAny = {fused}"
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
    put(
        ALT,
        &[
            0x1111_1111_2222_2222,
            0xA5A5_A5A5_5A5A_5A5A,
            0x3333_3333_4444_4444,
        ],
    );
    put(PTRBUF, &[ALT, 0x0F0E_0D0C_0B0A_0908]);
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(0), code.len() as u64).unwrap(),
        &code,
    )
    .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);
    for (n, v) in regs.into_iter().enumerate() {
        unit.state_mut().set_gpr(4 + n, v);
    }
    unit.run_until_yield(
        Budget::new(2),
        &ExecutionContext::new(&mem),
        &mut Vec::new(),
    );
    assert_eq!(unit.retired_instructions(), 2, "the case ran to its end");
    let g = &unit.state().gpr;
    [g[4], g[5], g[6], g[7]]
}

#[test]
fn every_ld_any_pair_case_leaves_the_registers_the_console_left() {
    let w = console_words();
    assert_eq!(w.len(), 22, "payload words");
    assert_eq!(w[0], 0, "the probe ran to its end");

    let [_, r5, r6, _] = run(
        [ld(5, 4, 8), ld(6, 7, 8)],
        true,
        [BUF, ALL_ONES, ALL_ONES, ALT],
    );
    assert_eq!(r5, console_reg(&w, 2), "two bases, first load");
    assert_eq!(r6, console_reg(&w, 4), "two bases, second load");

    let [_, r5, r6, _] = run(
        [ld(5, 4, 0), ld(6, 4, 24)],
        true,
        [BUF, ALL_ONES, ALL_ONES, 0],
    );
    assert_eq!(r5, console_reg(&w, 6), "one base 24 apart, first load");
    assert_eq!(r6, console_reg(&w, 8), "one base 24 apart, second load");

    let [r4, _, r6, _] = run(
        [ld(4, 4, 8), ld(6, 7, 16)],
        true,
        [PTRBUF, 0, ALL_ONES, BUF],
    );
    assert_eq!(r4, console_reg(&w, 10), "a first load into its own base");
    assert_eq!(r6, console_reg(&w, 12), "the second load from another base");

    let [_, r5, _, _] = run([ld(5, 4, 16), ld(5, 4, 0)], true, [BUF, ALL_ONES, 0, 0]);
    assert_eq!(r5, console_reg(&w, 14), "one register, the second value");

    let [_, r5, r6, _] = run(
        [ld(5, 4, -16), ld(6, 7, -8)],
        true,
        [BUF + 24, ALL_ONES, ALL_ONES, ALT + 24],
    );
    assert_eq!(r5, console_reg(&w, 16), "negative pair, first load");
    assert_eq!(r6, console_reg(&w, 18), "negative pair, second load");

    let [_, _, r6, _] = run([ld(7, 4, 0), ld(6, 7, 16)], false, [PTRBUF, 0, ALL_ONES, 0]);
    assert_eq!(
        r6,
        console_reg(&w, 20),
        "the second load reads the base the first load wrote"
    );
}
