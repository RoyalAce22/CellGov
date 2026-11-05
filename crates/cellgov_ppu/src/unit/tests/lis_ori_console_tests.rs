//! `lis` then `ori` on the fused path, held to the `ppu_lis_ori_pair`
//! capture from the reference console.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use crate::instruction::PpuInstruction;

/// The probe's payload, as big-endian u32 words.
fn console_words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_lis_ori_pair/ps3/cech20-cex-493/cgov_frame.bin");
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

/// GPR `reg` after `words` run from address 0 for two instructions,
/// from GPR `reg` = `start`. `fused` is whether the slot at `second`
/// holds the second half of a fused pair.
fn run(words: &[u32], second: u64, fused: bool, reg: usize, start: u64) -> u64 {
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
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(0), code.len() as u64).unwrap(),
        &code,
    )
    .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);
    unit.state_mut().set_gpr(reg, start);
    unit.run_until_yield(
        Budget::new(2),
        &ExecutionContext::new(&mem),
        &mut Vec::new(),
    );
    assert_eq!(unit.retired_instructions(), 2, "the case ran to its end");
    unit.state().gpr[reg]
}

const fn lis(rt: u32, si: u32) -> u32 {
    (15 << 26) | (rt << 21) | si
}

const fn ori(r: u32, ui: u32) -> u32 {
    (24 << 26) | (r << 21) | (r << 16) | ui
}

const B_OVER_ONE: u32 = (18 << 26) | 8;

#[test]
fn every_lis_ori_case_leaves_the_register_the_console_left() {
    let w = console_words();
    assert_eq!(w.len(), 14, "payload words");
    assert_eq!(w[0], 0, "the probe ran to its end");

    let r5 = run(&[lis(5, 0x8001), ori(5, 0x00F0)], 4, true, 5, 0);
    assert_eq!(r5, console_reg(&w, 2), "hi 0x8001");

    let r5 = run(&[lis(5, 0x7FFF), ori(5, 0xFFFF)], 4, true, 5, 0);
    assert_eq!(r5, console_reg(&w, 4), "hi 0x7FFF");

    let r5 = run(&[lis(5, 0xFFFF), ori(5, 0xFFFF)], 4, true, 5, 0);
    assert_eq!(r5, console_reg(&w, 6), "hi 0xFFFF");

    let code = [B_OVER_ONE, lis(5, 0x1234), ori(5, 0x00F0)];
    let r5 = run(&code, 8, true, 5, 0x0000_0001_0000_0100);
    assert_eq!(r5, console_reg(&w, 8), "a branch into the ori");

    let r0 = run(&[lis(0, 0x8000), ori(0, 0)], 4, false, 0, 0);
    assert_eq!(r0, console_reg(&w, 10), "r0, lo 0");

    let r0 = run(&[lis(0, 0x8000), ori(0, 1)], 4, true, 0, 0);
    assert_eq!(r0, console_reg(&w, 12), "r0, lo 1");
}
