//! Two adjacent `rldicl` on the fused path, held to the
//! `ppu_rldicl_rldicl_pair` capture from the reference console.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use crate::instruction::encode::encode;
use crate::instruction::{FusedPair, PpuInstruction};

/// The probe's inputs A, N and C.
const A: u64 = 0x0123_4567_89AB_CDEF;
const N: u64 = 0x8000_0000_0000_0001;
const C: u64 = 0x0000_0000_CAFE_F00D;

/// The CR0 field the probe sets before each case.
const CR0_PRESET: u8 = 0xF;
/// The XER the probe sets before its last case; the other cases set 0.
const XER_SO: u64 = 1 << 31;

/// The probe's payload, as big-endian u32 words.
fn console_words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_rldicl_rldicl_pair/ps3/cech20-cex-493/cgov_frame.bin");
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

/// The CR0 field the console recorded at payload word `at`.
fn console_cr0(w: &[u32], at: usize) -> u8 {
    u8::try_from(w[at]).expect("a 4-bit CR0 field")
}

/// `rldicl rA, rS, sh, mb` with Rc `rc`; `srdi` and `clrldi` are this
/// word with their own `sh` and `mb`.
fn rldicl(ra: u8, rs: u8, sh: u8, mb: u8, rc: bool) -> u32 {
    encode(&PpuInstruction::Rldicl { ra, rs, sh, mb, rc }).expect("rldicl encodes")
}

const B_OVER_ONE: u32 = (18 << 26) | 8;

/// Runs `steps` instructions of `words` and returns GPRs 0..=6 and CR0.
///
/// The run starts at address 0 with GPRs 0..=6 = `regs`, CR0 =
/// [`CR0_PRESET`] and XER = `xer`.
///
/// # Panics
///
/// Panics if:
/// - the slot at `pair` does not hold a fused `rldicl; rldicl`
/// - the run retires a count other than `steps`
fn run(words: &[u32], pair: u64, steps: u64, regs: [u64; 7], xer: u64) -> ([u64; 7], u8) {
    let code = words
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    let shadow = shadow::PredecodedShadow::build(0, &code);
    assert!(
        matches!(
            shadow.get(pair),
            Some(PpuInstruction::Fused(FusedPair::RldiclRldicl { .. }))
        ),
        "precondition: the slot at 0x{pair:x} is a fused rldicl; rldicl"
    );
    let mut mem = GuestMemory::new(0x100);
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(0), code.len() as u64).unwrap(),
        &code,
    )
    .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);
    for (n, v) in regs.into_iter().enumerate() {
        unit.state_mut().set_gpr(n, v);
    }
    unit.state_mut().set_cr_field(0, CR0_PRESET);
    unit.state_mut().set_xer(xer);
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
    let s = unit.state();
    (std::array::from_fn(|n| s.gpr[n]), s.cr_field(0))
}

#[test]
fn every_rldicl_pair_case_leaves_the_registers_and_cr0_the_console_left() {
    let w = console_words();
    assert_eq!(w.len(), 33, "payload words");
    assert_eq!(w[0], 0, "the probe ran to its end");

    let regs = [0, 0, 0, 0, A, 0, 0];
    let (g, cr0) = run(
        &[rldicl(3, 4, 8, 4, false), rldicl(5, 3, 0, 32, true)],
        0,
        2,
        regs,
        0,
    );
    assert_eq!(g[3], console_reg(&w, 2), "chained pair, first half");
    assert_eq!(g[5], console_reg(&w, 4), "chained pair, second half");
    assert_eq!(cr0, console_cr0(&w, 6), "chained pair, second half records");

    let (g, cr0) = run(
        &[rldicl(3, 4, 60, 4, false), rldicl(3, 3, 0, 32, false)],
        0,
        2,
        regs,
        0,
    );
    assert_eq!(
        g[3],
        console_reg(&w, 7),
        "srdi then clrldi into one register"
    );
    assert_eq!(cr0, console_cr0(&w, 9), "neither half records");

    let regs_n = [0, 0, 0, 0, N, 0, 0];
    let (g, cr0) = run(
        &[rldicl(3, 4, 0, 0, true), rldicl(5, 3, 62, 63, false)],
        0,
        2,
        regs_n,
        0,
    );
    assert_eq!(g[3], console_reg(&w, 10), "first half records, its result");
    assert_eq!(
        g[5],
        console_reg(&w, 12),
        "first half records, second result"
    );
    assert_eq!(cr0, console_cr0(&w, 14), "only the first half records");

    let (g, cr0) = run(
        &[rldicl(3, 4, 0, 0, true), rldicl(5, 3, 62, 63, true)],
        0,
        2,
        regs_n,
        0,
    );
    assert_eq!(g[3], console_reg(&w, 15), "both record, first result");
    assert_eq!(g[5], console_reg(&w, 17), "both record, second result");
    assert_eq!(cr0, console_cr0(&w, 19), "both record; the second wins");

    let (g, _) = run(
        &[
            B_OVER_ONE,
            rldicl(5, 4, 8, 4, false),
            rldicl(6, 5, 0, 32, false),
        ],
        4,
        2,
        [0, 0, 0, 0, A, C, 0],
        0,
    );
    assert_eq!(g[5], console_reg(&w, 20), "a branch over the first half");
    assert_eq!(g[6], console_reg(&w, 22), "the second half alone");

    let (g, _) = run(
        &[rldicl(3, 4, 63, 0, false), rldicl(4, 3, 1, 0, false)],
        0,
        2,
        regs,
        0,
    );
    assert_eq!(g[3], console_reg(&w, 24), "rotate by 63");
    assert_eq!(
        g[4],
        console_reg(&w, 26),
        "the second half writes the first half's source"
    );

    let (g, cr0) = run(
        &[rldicl(3, 4, 60, 4, true), rldicl(5, 3, 0, 32, true)],
        0,
        2,
        regs,
        XER_SO,
    );
    assert_eq!(g[3], console_reg(&w, 28), "srdi. under XER[SO]");
    assert_eq!(g[5], console_reg(&w, 30), "clrldi. under XER[SO]");
    assert_eq!(cr0, console_cr0(&w, 32), "CR0 SO copies XER[SO]");
}
