//! `or` then `add` on the fused path, held to the `ppu_or_add_pair`
//! capture from the reference console.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use crate::instruction::PpuInstruction;
use crate::state::PpuState;

/// The probe's payload, as big-endian u32 words.
fn console_words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_or_add_pair/ps3/cech20-cex-493/cgov_frame.bin");
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

/// The unit's state after `words` run from address 0 for two
/// instructions, from the GPRs `regs` sets. The slot at `second` must
/// hold the second half of a fused pair.
fn run(words: &[u32], second: u64, regs: &[(usize, u64)]) -> PpuState {
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
    let mut mem = GuestMemory::new(0x1000);
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(0), code.len() as u64).unwrap(),
        &code,
    )
    .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);
    for &(reg, value) in regs {
        unit.state_mut().set_gpr(reg, value);
    }
    unit.run_until_yield(
        Budget::new(2),
        &ExecutionContext::new(&mem),
        &mut Vec::new(),
    );
    assert_eq!(unit.retired_instructions(), 2, "the case ran to its end");
    unit.state().clone()
}

const fn or(ra: u32, rs: u32, rb: u32) -> u32 {
    (31 << 26) | (rs << 21) | (ra << 16) | (rb << 11) | (444 << 1)
}

const fn add(rt: u32, ra: u32, rb: u32) -> u32 {
    (31 << 26) | (rt << 21) | (ra << 16) | (rb << 11) | (266 << 1)
}

const B_OVER_ONE: u32 = (18 << 26) | 8;

const HI_BIT: u64 = 0x8000_0001_0000_0000;
const LOW_ONES: u64 = 0x0000_0000_FFFF_FFFF;

#[test]
fn every_or_add_case_leaves_the_registers_the_console_left() {
    let w = console_words();
    assert_eq!(w.len(), 18, "payload words");
    assert_eq!(w[0], 0, "the probe ran to its end");

    let s = run(
        &[or(5, 6, 7), add(8, 5, 9)],
        4,
        &[(6, HI_BIT), (7, LOW_ONES), (9, 1)],
    );
    assert_eq!(s.gpr[5], console_reg(&w, 2), "or + add: r5");
    assert_eq!(s.gpr[8], console_reg(&w, 4), "or + add: r8");

    let s = run(&[or(5, 6, 6), add(8, 5, 9)], 4, &[(6, u64::MAX), (9, 2)]);
    assert_eq!(s.gpr[5], console_reg(&w, 6), "mr + add: r5");
    assert_eq!(s.gpr[8], console_reg(&w, 8), "mr + add: r8");

    let s = run(
        &[B_OVER_ONE, or(5, 6, 7), add(8, 5, 9)],
        8,
        &[(5, 0x0000_0001_0000_0100), (6, 0x10), (7, 0x20), (9, 3)],
    );
    assert_eq!(s.gpr[5], console_reg(&w, 10), "a branch into the add: r5");
    assert_eq!(s.gpr[8], console_reg(&w, 12), "a branch into the add: r8");

    let s = run(
        &[or(5, 6, 7), add(5, 5, 5)],
        4,
        &[(6, 0x4000_0000_0000_0000), (7, 1)],
    );
    assert_eq!(s.gpr[5], console_reg(&w, 14), "add into the or's RA: r5");

    let s = run(
        &[or(5, 6, 7), add(8, 9, 5)],
        4,
        &[(6, HI_BIT), (7, LOW_ONES), (9, 1)],
    );
    assert_eq!(s.gpr[8], console_reg(&w, 16), "add reading RB: r8");
}
