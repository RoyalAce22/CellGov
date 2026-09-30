//! The logical completion: operand order in the complement forms, the three
//! immediate widths, and orx's zeroed slots.

use super::*;
use crate::state::SpuState;

/// [SPU-ISA p:28 s:2.3] RR: 11-bit opcode, RB, RA, RT.
fn rr(op: u32, rt: u32, ra: u32, rb: u32) -> u32 {
    op << 21 | rb << 14 | ra << 7 | rt
}

/// [SPU-ISA p:29 s:2.3] RI10: 8-bit opcode, I10, RA, RT.
fn ri10(op: u32, rt: u32, ra: u32, i10: u32) -> u32 {
    op << 24 | (i10 & 0x3FF) << 14 | ra << 7 | rt
}

const ANDC: u32 = 0x2C1;
const ORC: u32 = 0x2C9;
const XOR: u32 = 0x241;
const NAND: u32 = 0x0C9;
const EQV: u32 = 0x249;
const ORX: u32 = 0x1F0;
const ANDBI: u32 = 0x16;
const ANDHI: u32 = 0x15;
const ORBI: u32 = 0x06;
const ORHI: u32 = 0x05;
const XORBI: u32 = 0x46;
const XORHI: u32 = 0x45;
const XORI: u32 = 0x44;

/// Runs `raw` with `a` in r1, `b` in r2 and junk in r3, and returns r3's words.
fn run(raw: u32, a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    let mut s = SpuState::new();
    s.regs[1] = from_words(a);
    s.regs[2] = from_words(b);
    s.regs[3] = [0xAA; 16];
    let insn = crate::decode::decode(raw).expect("decodes");
    assert_eq!(
        execute(&insn, &mut s, UnitId::new(0)),
        SpuStepOutcome::Continue
    );
    words(s.regs[3])
}

#[test]
fn the_register_forms_complement_rb_not_ra() {
    // Nibbles a = 1100 and b = 1010: every form gives a different nibble, and
    // swapping the operands changes andc and orc.
    let (a, b) = ([0xCCCC_CCCC; 4], [0xAAAA_AAAA; 4]);
    for (op, want) in [
        (ANDC, 0x4444_4444),
        (ORC, 0xDDDD_DDDD),
        (XOR, 0x6666_6666),
        (NAND, 0x7777_7777),
        (EQV, 0x9999_9999),
    ] {
        assert_eq!(run(rr(op, 3, 1, 2), a, b), [want; 4], "opcode {op:#05x}");
    }
}

#[test]
fn the_immediate_widths_differ_when_i10_bit_9_is_set() {
    // I10 0x2F0: the byte forms use 0xF0, the halfword forms 0xFEF0, xori 0xFFFF_FEF0.
    let ones = [u32::MAX; 4];
    let zeros = [0; 4];
    let mixed = [0x1234_5678; 4];
    for (op, a, want) in [
        (ANDBI, ones, 0xF0F0_F0F0),
        (ANDHI, ones, 0xFEF0_FEF0),
        (ORBI, zeros, 0xF0F0_F0F0),
        (ORHI, zeros, 0xFEF0_FEF0),
        (XORBI, mixed, 0xE2C4_A688),
        (XORHI, mixed, 0xECC4_A888),
        (XORI, mixed, 0xEDCB_A888),
    ] {
        assert_eq!(
            run(ri10(op, 3, 1, 0x2F0), a, zeros),
            [want; 4],
            "opcode {op:#04x}"
        );
    }
}

#[test]
fn orx_ors_the_words_into_the_preferred_slot_and_zeroes_the_rest() {
    assert_eq!(
        run(rr(ORX, 3, 1, 0), [0x1, 0x20, 0x300, 0x4000_0000], [0; 4]),
        [0x4000_0321, 0, 0, 0]
    );
}

#[test]
fn the_byte_immediate_forms_decode_only_the_rightmost_8_bits() {
    for op in [ANDBI, ORBI, XORBI] {
        assert_eq!(
            crate::decode::decode(ri10(op, 3, 1, 0x3F0)),
            crate::decode::decode(ri10(op, 3, 1, 0x0F0)),
            "opcode {op:#04x}"
        );
    }
}
