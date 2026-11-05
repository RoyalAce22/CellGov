//! `lbzu` then `cmpwi crF, rC, 0` on the fused path, held to the
//! `ppu_lbzu_cmpwi_pair` capture from the reference console.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use crate::instruction::PpuInstruction;
use crate::state::PpuState;

/// The probe's `str` and `hi` addresses here.
const STR: u64 = 0x800;
const HI: u64 = 0x900;

/// XER with SO set.
const XER_SO: u64 = 0x8000_0000;

/// The probe's payload, as big-endian u32 words.
fn console_words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_lbzu_cmpwi_pair/ps3/cech20-cex-493/cgov_frame.bin");
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

/// `lbzu rT, d(rA)`.
const fn lbzu(rt: u32, ra: u32, d: i16) -> u32 {
    (35 << 26) | (rt << 21) | (ra << 16) | (d as u16 as u32)
}

/// `cmpwi crF, rA, 0`.
const fn cmpwi0(bf: u32, ra: u32) -> u32 {
    (11 << 26) | (bf << 23) | (ra << 16)
}

/// `bne crF, -8`: BO 4 branches when the field's EQ bit is 0.
const fn bne_back_two(bf: u32) -> u32 {
    (16 << 26) | (4 << 21) | ((4 * bf + 2) << 16) | (-8i16 as u16 as u32)
}

const B_OVER_ONE: u32 = (18 << 26) | 8;

/// Runs `words` from address 0 for `steps` instructions and returns the state.
///
/// The run starts from the GPRs `regs` and the XER `xer`. Panics if the
/// slot at `second` does not hold the second half of a fused pair.
fn run(words: &[u32], second: u64, steps: u64, regs: &[(usize, u64)], xer: u64) -> PpuState {
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
    for (addr, bytes) in [
        (0, code.as_slice()),
        (STR, b"abc\0"),
        (HI, &[0x80, 0, 0, 0]),
    ] {
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
    assert_eq!(
        unit.state().pc,
        4 * words.len() as u64,
        "the case left its code"
    );
    unit.state().clone()
}

#[test]
fn every_lbzu_cmpwi_case_leaves_what_the_console_left() {
    let w = console_words();
    assert_eq!(w.len(), 21, "payload words");
    assert_eq!(w[0], 0, "the probe ran to its end");

    // Four bytes, each one lbzu, cmpwi and bne.
    let code = [lbzu(4, 5, 1), cmpwi0(7, 4), bne_back_two(7)];
    let s = run(&code, 4, 12, &[(4, u64::MAX), (5, STR - 1)], 0);
    assert_eq!(s.gpr[4], console_reg(&w, 2), "scan: r4");
    assert_eq!(
        s.gpr[5] - (STR - 1),
        console_reg(&w, 4),
        "scan: r5 distance"
    );
    assert_eq!(u32::from(s.cr_field(7)), w[6], "scan: cr7");

    let code = [lbzu(4, 5, 1), cmpwi0(0, 4)];
    let s = run(&code, 4, 2, &[(4, u64::MAX), (5, HI - 1)], XER_SO);
    assert_eq!(s.gpr[4], console_reg(&w, 7), "0x80 with SO: r4");
    assert_eq!(
        s.gpr[5] - (HI - 1),
        console_reg(&w, 9),
        "0x80 with SO: r5 distance"
    );
    assert_eq!(u32::from(s.cr_field(0)), w[11], "0x80 with SO: cr0");

    assert_eq!(
        (HI as u32) >> 31,
        w[13],
        "precondition: hi's low word has the console's sign"
    );
    let s = run(&[lbzu(4, 5, 1), cmpwi0(7, 5)], 4, 2, &[(5, HI - 1)], 0);
    assert_eq!(u32::from(s.cr_field(7)), w[12], "compare of the base: cr7");

    let regs = [(5, HI - 1), (6, 0xFFFF_FFFF_0000_0000)];
    let s = run(&[lbzu(4, 5, 1), cmpwi0(1, 6)], 4, 2, &regs, 0);
    assert_eq!(u32::from(s.cr_field(1)), w[14], "low word zero: cr1");

    let regs = [(5, HI - 1), (6, 0x0000_0000_8000_0000)];
    let s = run(&[lbzu(4, 5, 1), cmpwi0(7, 6)], 4, 2, &regs, 0);
    assert_eq!(u32::from(s.cr_field(7)), w[15], "low word negative: cr7");

    let code = [B_OVER_ONE, lbzu(4, 5, 1), cmpwi0(7, 4)];
    let s = run(&code, 8, 2, &[(4, 0x44), (5, HI - 1)], 0);
    assert_eq!(
        s.gpr[4],
        console_reg(&w, 16),
        "a branch into the compare: r4"
    );
    assert_eq!(
        s.gpr[5] - (HI - 1),
        console_reg(&w, 18),
        "a branch into the compare: r5 distance"
    );
    assert_eq!(
        u32::from(s.cr_field(7)),
        w[20],
        "a branch into the compare: cr7"
    );
}
