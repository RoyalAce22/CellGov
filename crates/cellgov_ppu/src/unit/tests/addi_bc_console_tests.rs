//! An adjacent `addi; bc` on the fused path, held to the
//! `ppu_addi_bc_pair` capture from the reference console.

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
        .join("../../tests/micro/ppu_addi_bc_pair/ps3/cech20-cex-493/cgov_frame.bin");
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

const fn addi(rt: u32, ra: u32, imm: i16) -> u32 {
    (14 << 26) | (rt << 21) | (ra << 16) | (imm as u16 as u32)
}

/// `bc bo, bi, offset` with AA = 0 and LK = 0.
const fn bc(bo: u32, bi: u32, offset: i16) -> u32 {
    (16 << 26) | (bo << 21) | (bi << 16) | (offset as u16 as u32)
}

const BDNZ: u32 = 0b10000;
const BNE: u32 = 0b00100;
const BEQ: u32 = 0b01100;
const B_OVER_ONE: u32 = (18 << 26) | 8;

/// Runs `words` from address 0 for `steps` instructions with CR 0, CTR
/// `ctr` and the GPRs `regs`.
///
/// # Panics
///
/// Panics if:
/// - the slot at `first` does not hold a fused `addi; bc`
/// - the run retires fewer than `steps` instructions
/// - the run does not stop at the end of `words`
fn run(words: &[u32], first: u64, steps: u64, ctr: u64, regs: &[(usize, u64)]) -> PpuState {
    let code = words
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    let shadow = shadow::PredecodedShadow::build(0, &code);
    assert!(
        matches!(
            shadow.get(first),
            Some(PpuInstruction::Fused(FusedPair::AddiBc { .. }))
        ),
        "precondition: the addi and the bc fuse at 0x{first:x}"
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
    unit.state_mut().set_ctr(ctr);
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
fn every_addi_bc_pair_case_leaves_what_the_console_left() {
    let w = console_words();
    assert_eq!(w.len(), 24, "payload words");
    assert_eq!(w[0], 0, "the probe ran to its end");

    let s = run(&[addi(5, 5, 1), bc(BDNZ, 0, -4)], 0, 6, 3, &[]);
    assert_eq!(s.gpr[5], console_reg(&w, 2), "counted loop: r5");
    assert_eq!(s.ctr(), console_reg(&w, 4), "counted loop: CTR");

    let code = [addi(5, 0, -3), bc(BNE, 2, 8), addi(7, 0, 1)];
    let s = run(&code, 0, 2, 0, &[(0, 0x100)]);
    assert_eq!(s.gpr[5], console_reg(&w, 6), "li with r0 nonzero: r5");
    assert_eq!(s.gpr[7], console_reg(&w, 8), "li with r0 nonzero: r7");

    let code = [addi(5, 6, -3), bc(BEQ, 2, 8), addi(7, 0, 1)];
    let s = run(&code, 0, 3, 0, &[(6, 1)]);
    assert_eq!(s.gpr[5], console_reg(&w, 10), "wrap and beq: r5");
    assert_eq!(s.gpr[7], console_reg(&w, 12), "wrap and beq: r7");

    let s = run(&[addi(5, 5, 1), bc(BDNZ, 0, 0)], 0, 4, 3, &[]);
    assert_eq!(s.gpr[5], console_reg(&w, 14), "bdnz to its own address: r5");
    assert_eq!(s.ctr(), console_reg(&w, 16), "bdnz to its own address: CTR");

    let code = [B_OVER_ONE, addi(5, 5, 1), bc(BDNZ, 0, -8), addi(6, 6, 1)];
    let s = run(&code, 4, 7, 3, &[]);
    assert_eq!(s.gpr[5], console_reg(&w, 18), "a branch into the bdnz: r5");
    assert_eq!(s.gpr[6], console_reg(&w, 20), "a branch into the bdnz: r6");
    assert_eq!(s.ctr(), console_reg(&w, 22), "a branch into the bdnz: CTR");
}
