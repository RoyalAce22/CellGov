//! Byte, halfword and word compares: the signed / unsigned boundary at each
//! width, and the three immediate widths.

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

const CEQB: u32 = 0x3D0;
const CEQH: u32 = 0x3C8;
const CEQHI: u32 = 0x7D;
const CGTB: u32 = 0x250;
const CGTBI: u32 = 0x4E;
const CGTH: u32 = 0x248;
const CGTHI: u32 = 0x4D;
const CGT: u32 = 0x240;
const CLGTB: u32 = 0x2D0;
const CLGTBI: u32 = 0x5E;
const CLGTH: u32 = 0x2C8;
const CLGTHI: u32 = 0x5D;
const CLGTI: u32 = 0x5C;

/// Runs `raw` with `a` in r1, `b` in r2 and junk in r3, and returns r3.
fn run(raw: u32, a: [u8; 16], b: [u8; 16]) -> [u8; 16] {
    let mut s = SpuState::new();
    s.regs[1] = a;
    s.regs[2] = b;
    s.regs[3] = [0xAA; 16];
    let insn = crate::decode::decode(raw).expect("decodes");
    assert_eq!(
        execute(&insn, &mut s, UnitId::new(0)),
        SpuStepOutcome::Continue
    );
    s.regs[3]
}

fn bytes(head: &[u8]) -> [u8; 16] {
    std::array::from_fn(|i| head.get(i).copied().unwrap_or(0))
}

fn halves(head: &[u16]) -> [u8; 16] {
    from_halfwords(std::array::from_fn(|i| head.get(i).copied().unwrap_or(0)))
}

#[test]
fn the_byte_compares_split_at_0x80() {
    let a = bytes(&[0x80, 0x7F, 0x01, 0xFF, 0x00]);
    let b = bytes(&[0x7F, 0x80, 0x01, 0x00, 0xFF]);
    let mut equal = [0xFF; 16];
    equal[..5].copy_from_slice(&[0, 0, 0xFF, 0, 0]);
    assert_eq!(run(rr(CEQB, 3, 1, 2), a, b), equal);
    assert_eq!(run(rr(CGTB, 3, 1, 2), a, b), bytes(&[0, 0xFF, 0, 0, 0xFF]));
    assert_eq!(run(rr(CLGTB, 3, 1, 2), a, b), bytes(&[0xFF, 0, 0, 0xFF, 0]));
}

#[test]
fn the_halfword_compares_split_at_0x8000() {
    let a = halves(&[0x8000, 0x7FFF, 0x0001, 0xFFFF, 0x0000]);
    let b = halves(&[0x7FFF, 0x8000, 0x0001, 0x0000, 0xFFFF]);
    let equal = halves(&[0, 0, 0xFFFF, 0, 0, 0xFFFF, 0xFFFF, 0xFFFF]);
    assert_eq!(run(rr(CEQH, 3, 1, 2), a, b), equal);
    assert_eq!(
        run(rr(CGTH, 3, 1, 2), a, b),
        halves(&[0, 0xFFFF, 0, 0, 0xFFFF])
    );
    assert_eq!(
        run(rr(CLGTH, 3, 1, 2), a, b),
        halves(&[0xFFFF, 0, 0, 0xFFFF, 0])
    );
}

#[test]
fn cgt_is_signed_at_0x80000000() {
    let a = from_words([0x8000_0000, 0x7FFF_FFFF, 1, u32::MAX]);
    let b = from_words([0x7FFF_FFFF, 0x8000_0000, 1, 0]);
    assert_eq!(run(rr(CGT, 3, 1, 2), a, b), from_words([0, u32::MAX, 0, 0]));
}

#[test]
fn the_byte_immediates_read_the_rightmost_8_bits_of_i10() {
    let a = bytes(&[0x80, 0x7F, 0x00, 0xFF]);
    // I10 0x380 and 0x37F: bits 8 and 9 set, rightmost bytes 0x80 and 0x7F.
    let mut above_min = [0xFF; 16];
    above_min[0] = 0;
    assert_eq!(run(ri10(CGTBI, 3, 1, 0x380), a, [0; 16]), above_min);
    assert_eq!(
        run(ri10(CLGTBI, 3, 1, 0x37F), a, [0; 16]),
        bytes(&[0xFF, 0, 0, 0xFF])
    );
}

#[test]
fn the_halfword_immediates_sign_extend_i10_to_16_bits() {
    // I10 0x200 is 0xFE00 as a halfword: -512 signed, 65024 unsigned.
    let a = halves(&[0x8000, 0xFE00, 0xFE01, 0x0000]);
    let mut signed = [0xFF; 16];
    signed[..4].copy_from_slice(&[0, 0, 0, 0]);
    assert_eq!(run(ri10(CGTHI, 3, 1, 0x200), a, [0; 16]), signed);
    assert_eq!(
        run(ri10(CLGTHI, 3, 1, 0x200), a, [0; 16]),
        halves(&[0, 0, 0xFFFF])
    );
    assert_eq!(
        run(ri10(CEQHI, 3, 1, 0x200), a, [0; 16]),
        halves(&[0, 0xFFFF])
    );
}

#[test]
fn clgti_compares_a_negative_i10_unsigned() {
    // I10 0x200 is 0xFFFF_FE00 as a word.
    let a = from_words([0xFFFF_FE01, 0xFFFF_FE00, 0x8000_0000, 1]);
    assert_eq!(
        run(ri10(CLGTI, 3, 1, 0x200), a, [0; 16]),
        from_words([u32::MAX, 0, 0, 0])
    );
}
