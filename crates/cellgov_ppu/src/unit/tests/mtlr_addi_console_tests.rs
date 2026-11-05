//! An `mtlr` followed by an `addi` on the fused path, held to the
//! `ppu_mtlr_addi_pair` capture from the reference console.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use crate::instruction::{FusedPair, PpuInstruction};

/// The probe's inputs A, B and C.
const A: u64 = 0x0123_4567_89AB_CDEF;
const B: u64 = 0xFEDC_BA98_7654_3210;
const C: u64 = 0x0000_0000_CAFE_F00D;

/// The probe's payload, as big-endian u32 words.
fn console_words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_mtlr_addi_pair/ps3/cech20-cex-493/cgov_frame.bin");
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

/// `mtlr rS`.
const fn mtlr(rs: u32) -> u32 {
    (31 << 26) | (rs << 21) | (8 << 16) | (467 << 1)
}

/// `addi rT, rA, si`; `li rT, si` when `ra` is 0.
const fn addi(rt: u32, ra: u32, si: i16) -> u32 {
    (14 << 26) | (rt << 21) | (ra << 16) | (si as u16 as u32)
}

const B_OVER_ONE: u32 = (18 << 26) | 8;

/// LR and GPRs 0..=6 after `words` run from address 0 for `steps`
/// instructions, from GPRs 0..=6 = `regs` and LR = `lr`.
///
/// # Panics
///
/// Panics if:
/// - the slot at `pair` does not hold a fused `mtlr; addi`
/// - the run retires a count other than `steps`
fn run(words: &[u32], pair: u64, steps: u64, regs: [u64; 7], lr: u64) -> (u64, [u64; 7]) {
    let code = words
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    let shadow = shadow::PredecodedShadow::build(0, &code);
    assert!(
        matches!(
            shadow.get(pair),
            Some(PpuInstruction::Fused(FusedPair::MtlrAddi { .. }))
        ),
        "precondition: the slot at 0x{pair:x} is a fused mtlr; addi"
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
    unit.state_mut().set_lr(lr);
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
    (s.lr(), std::array::from_fn(|n| s.gpr[n]))
}

#[test]
fn every_mtlr_addi_case_leaves_the_lr_and_register_the_console_left() {
    let w = console_words();
    assert_eq!(w.len(), 30, "payload words");
    assert_eq!(w[0], 0, "the probe ran to its end");

    let (lr, g) = run(&[mtlr(3), addi(5, 4, -2)], 0, 2, [0, 0, 0, A, B, 0, 0], 0);
    assert_eq!(lr, console_reg(&w, 2), "plain pair, LR");
    assert_eq!(g[5], console_reg(&w, 4), "plain pair, addi result");

    let (lr, g) = run(&[mtlr(3), addi(5, 0, -1)], 0, 2, [0, 0, 0, A, 0, B, 0], 0);
    assert_eq!(lr, console_reg(&w, 6), "li form, LR");
    assert_eq!(g[5], console_reg(&w, 8), "li form, li result");

    let (lr, g) = run(&[mtlr(3), addi(3, 3, 16)], 0, 2, [0, 0, 0, A, 0, 0, 0], 0);
    assert_eq!(
        lr,
        console_reg(&w, 10),
        "the mtlr source before the addi overwrites it"
    );
    assert_eq!(g[3], console_reg(&w, 12), "an addi into the mtlr source");

    let (lr, g) = run(
        &[B_OVER_ONE, mtlr(3), addi(5, 4, 1)],
        4,
        2,
        [0, 0, 0, A, B, 0, 0],
        C,
    );
    assert_eq!(lr, console_reg(&w, 14), "a branch over the mtlr leaves LR");
    assert_eq!(g[5], console_reg(&w, 16), "the addi alone");

    let (lr, g) = run(&[mtlr(0), addi(5, 0, 7)], 0, 2, [A, 0, 0, 0, 0, B, 0], 0);
    assert_eq!(
        lr,
        console_reg(&w, 18),
        "r0 as the mtlr source is the register"
    );
    assert_eq!(g[5], console_reg(&w, 20), "li after mtlr r0");

    let (lr, g) = run(
        &[mtlr(3), addi(5, 4, 1)],
        0,
        2,
        [0, 0, 0, A, i64::MAX as u64, 0, 0],
        0,
    );
    assert_eq!(lr, console_reg(&w, 22), "signed-maximum wrap, LR");
    assert_eq!(g[5], console_reg(&w, 24), "an add past the signed maximum");

    let (lr, g) = run(
        &[mtlr(3), addi(4, 4, 1)],
        0,
        2,
        [0, 0, 0, A, u64::MAX, 0, 0],
        0,
    );
    assert_eq!(lr, console_reg(&w, 26), "all-ones wrap, LR");
    assert_eq!(
        g[4],
        console_reg(&w, 28),
        "an add past all ones into its own base"
    );
}
