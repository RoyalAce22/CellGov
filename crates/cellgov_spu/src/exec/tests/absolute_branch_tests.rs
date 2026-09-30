//! Absolute branches: a positive target, a negative I16 wrapping through
//! the limit register to the top of local store, and brasl's link layout.

use super::*;
use crate::state::SpuState;

/// [SPU-ISA p:29 s:2.3] RI16: 9-bit opcode, I16, RT.
fn ri16(op: u32, rt: u32, i16: u32) -> u32 {
    op << 23 | (i16 & 0xFFFF) << 7 | rt
}

const BRA: u32 = 0x060;
const BRASL: u32 = 0x062;

fn step(s: &mut SpuState, raw: u32) -> SpuStepOutcome {
    let insn = crate::decode::decode(raw).expect("decodes");
    execute(&insn, s, UnitId::new(0))
}

#[test]
fn bra_jumps_to_the_word_address_regardless_of_the_pc() {
    let mut s = SpuState::new();
    s.pc = 0x1_0000;
    // [SPU-ISA p:175 s:7] bra's RT field is unused, so a set one still decodes.
    assert_eq!(
        step(&mut s, ri16(BRA, 0x7F, 0x0100)),
        SpuStepOutcome::Branch
    );
    assert_eq!(s.pc, 0x400);
}

#[test]
fn a_negative_i16_wraps_through_the_limit_register() {
    // I16 0xFFFF is -1, so the target is -4 masked to the last word of
    // local store.
    let mut s = SpuState::new();
    s.pc = 0x100;
    assert_eq!(step(&mut s, ri16(BRA, 0, 0xFFFF)), SpuStepOutcome::Branch);
    assert_eq!(s.pc, 0x3_FFFC);
}

#[test]
fn brasl_links_the_next_address_in_the_preferred_slot_only() {
    let mut s = SpuState::new();
    s.pc = 0x3_FFFC;
    s.regs[5] = [0xAA; 16];
    assert_eq!(step(&mut s, ri16(BRASL, 5, 0x0040)), SpuStepOutcome::Branch);
    assert_eq!(s.pc, 0x100);
    // PC + 4 wraps through the limit register to zero.
    assert_eq!(s.regs[5], [0; 16]);
    s.pc = 0x200;
    step(&mut s, ri16(BRASL, 5, 0x8000));
    assert_eq!(s.pc, 0x2_0000);
    assert_eq!(words(s.regs[5]), [0x204, 0, 0, 0]);
}
