//! Shift and rotate: the quadword, word and halfword forms.

use super::ls::{negated_count, rotate_mask_count};
use super::outcome::SpuStepOutcome;
use super::slots::{halfwords2, words2};
use crate::state::SpuState;

/// `half` shifted right `count` bits with its sign bit replicated; every bit
/// is the sign bit once the count exceeds 15.
fn shift_right_algebraic_halfword(half: u16, count: u32) -> u16 {
    let signed = half as i16;
    signed.checked_shr(count).unwrap_or(signed >> 15) as u16
}

/// `word` shifted right `count` bits with zero fill; zero once the count
/// exceeds 31.
fn shift_right_logical_word(word: u32, count: u32) -> u32 {
    word.checked_shr(count).unwrap_or(0)
}

/// `word` shifted right `count` bits with its sign bit replicated; every bit
/// is the sign bit once the count exceeds 31.
fn shift_right_algebraic_word(word: u32, count: u32) -> u32 {
    let signed = word as i32;
    signed.checked_shr(count).unwrap_or(signed >> 31) as u32
}

/// Applies `f` to `ra` as one big-endian 128-bit value and writes the
/// result to `rt`.
fn quad(state: &mut SpuState, rt: u8, ra: u8, f: impl Fn(u128) -> u128) -> SpuStepOutcome {
    let q = u128::from_be_bytes(state.regs[ra as usize]);
    state.set_reg(rt as usize, f(q).to_be_bytes());
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:125 s:6. Shift and Rotate Instructions] Shift Left Quadword by Bytes Immediate: shift register left by I7 bytes, fill zero.
pub(super) fn shlqbyi(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    let s = u32::from(imm & 0x1F);
    quad(state, rt, ra, |q| q.checked_shl(8 * s).unwrap_or(0))
}

/// [SPU-ISA p:131 s:6. Shift and Rotate Instructions] Rotate Quadword by Bytes: byte rotate count taken from low nibble of RB preferred slot.
pub(super) fn rotqby(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let s = state.reg_word_slot(rb, 0) & 0x0F;
    quad(state, rt, ra, |q| q.rotate_left(8 * s))
}

/// [SPU-ISA p:132 s:6. Shift and Rotate Instructions] Rotate Quadword by Bytes Immediate: byte rotate count is the low nibble of I7.
pub(super) fn rotqbyi(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    let s = u32::from(imm & 0x0F);
    quad(state, rt, ra, |q| q.rotate_left(8 * s))
}

/// [SPU-ISA p:141 s:6. Shift and Rotate Instructions] Rotate and Mask Quadword by Bytes Immediate: shift right by (0 - I7) mod 32 bytes with zero fill; a count of 16 or more clears the register.
pub(super) fn rotqmbyi(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    let s = negated_count(u32::from(imm), 0x1F);
    quad(state, rt, ra, |q| q.checked_shr(8 * s).unwrap_or(0))
}

/// [SPU-ISA p:122 s:6. Shift and Rotate Instructions] Shift Left Quadword by Bits: count is bits 29 to 31 of RB's preferred slot.
pub(super) fn shlqbi(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let s = state.reg_word_slot(rb, 0) & 0x07;
    quad(state, rt, ra, |q| q << s)
}

/// [SPU-ISA p:123 s:6. Shift and Rotate Instructions] Shift Left Quadword by Bits Immediate: count is the low 3 bits of I7.
pub(super) fn shlqbii(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    let s = u32::from(imm & 0x07);
    quad(state, rt, ra, |q| q << s)
}

/// [SPU-ISA p:134 s:6. Shift and Rotate Instructions] Rotate Quadword by Bits: count is bits 29 to 31 of RB's preferred slot.
pub(super) fn rotqbi(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let s = state.reg_word_slot(rb, 0) & 0x07;
    quad(state, rt, ra, |q| q.rotate_left(s))
}

/// [SPU-ISA p:135 s:6. Shift and Rotate Instructions] Rotate Quadword by Bits Immediate: count is the low 3 bits of I7.
pub(super) fn rotqbii(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    let s = u32::from(imm & 0x07);
    quad(state, rt, ra, |q| q.rotate_left(s))
}

/// [SPU-ISA p:143 s:6. Shift and Rotate Instructions] Rotate and Mask Quadword by Bits: logical right shift by (0 - RB's preferred slot) mod 8.
pub(super) fn rotqmbi(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let s = negated_count(state.reg_word_slot(rb, 0), 0x07);
    quad(state, rt, ra, |q| q >> s)
}

/// [SPU-ISA p:144 s:6. Shift and Rotate Instructions] Rotate and Mask Quadword by Bits Immediate: logical right shift by (0 - I7) mod 8.
pub(super) fn rotqmbii(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    let s = negated_count(u32::from(imm), 0x07);
    quad(state, rt, ra, |q| q >> s)
}

/// [SPU-ISA p:124 s:6. Shift and Rotate Instructions] Shift Left Quadword by Bytes: count is bits 27 to 31 of RB's preferred slot; a count above 15 yields zero.
pub(super) fn shlqby(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let s = state.reg_word_slot(rb, 0) & 0x1F;
    quad(state, rt, ra, |q| q.checked_shl(8 * s).unwrap_or(0))
}

/// [SPU-ISA p:126 s:6. Shift and Rotate Instructions] Shift Left Quadword by Bytes from Bit Shift Count: count is bits 24 to 28 of RB's preferred slot; a count above 15 yields zero.
pub(super) fn shlqbybi(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let s = (state.reg_word_slot(rb, 0) >> 3) & 0x1F;
    quad(state, rt, ra, |q| q.checked_shl(8 * s).unwrap_or(0))
}

/// [SPU-ISA p:133 s:6. Shift and Rotate Instructions] Rotate Quadword by Bytes from Bit Shift Count: the RTL reads bits 24 to 28 and rotates modulo 16, so bits 25 to 28 decide.
pub(super) fn rotqbybi(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let s = (state.reg_word_slot(rb, 0) >> 3) & 0x0F;
    quad(state, rt, ra, |q| q.rotate_left(8 * s))
}

/// [SPU-ISA p:140 s:6. Shift and Rotate Instructions] Rotate and Mask Quadword by Bytes: right shift by (0 - RB's preferred slot) mod 32 bytes; a count above 15 yields zero.
pub(super) fn rotqmby(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let s = negated_count(state.reg_word_slot(rb, 0), 0x1F);
    quad(state, rt, ra, |q| q.checked_shr(8 * s).unwrap_or(0))
}

/// [SPU-ISA p:142 s:6. Shift and Rotate Instructions] Rotate and Mask Quadword Bytes from Bit Shift Count: right shift by (0 - bits 24 to 28 of RB's preferred slot) mod 32 bytes.
pub(super) fn rotqmbybi(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let s = negated_count(state.reg_word_slot(rb, 0) >> 3, 0x1F);
    quad(state, rt, ra, |q| q.checked_shr(8 * s).unwrap_or(0))
}

/// [SPU-ISA p:120 s:6. Shift and Rotate Instructions] Shift Left Word: per-slot count is the low 6 bits of the RB slot; a count above 31 yields zero.
pub(super) fn shl(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    words2(state, rt, ra, rb, |a, b| {
        a.checked_shl(b & 0x3F).unwrap_or(0)
    })
}

/// [SPU-ISA p:121 s:6. Shift and Rotate Instructions] Shift Left Word Immediate: count is the low 6 bits of sign-extended I7; a count above 31 yields zero.
pub(super) fn shli(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    let s = u32::from(imm & 0x3F);
    words2(state, rt, ra, ra, |a, _| a.checked_shl(s).unwrap_or(0))
}

/// [SPU-ISA p:139 s:6. Shift and Rotate Instructions] Rotate and Mask Word Immediate: logical right shift by (0 - I7) mod 64; a count above 31 yields zero.
pub(super) fn rotmi(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    let s = rotate_mask_count(imm);
    words2(state, rt, ra, ra, |a, _| shift_right_logical_word(a, s))
}

/// [SPU-ISA p:148 s:6. Shift and Rotate Instructions] Rotate and Mask Algebraic Word Immediate: arithmetic right shift by (0 - I7) mod 64; a count above 31 fills with the sign bit.
pub(super) fn rotmai(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    let s = rotate_mask_count(imm);
    words2(state, rt, ra, ra, |a, _| shift_right_algebraic_word(a, s))
}

/// [SPU-ISA p:129 s:6. Shift and Rotate Instructions] Rotate Word: each word's count is bits 27 to 31 of its RB word.
pub(super) fn rot(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    words2(state, rt, ra, rb, |a, b| a.rotate_left(b & 0x1F))
}

/// [SPU-ISA p:130 s:6. Shift and Rotate Instructions] Rotate Word Immediate: count is the low 5 bits of sign-extended I7.
pub(super) fn roti(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    let s = u32::from(imm & 0x1F);
    words2(state, rt, ra, ra, |a, _| a.rotate_left(s))
}

/// [SPU-ISA p:138 s:6. Shift and Rotate Instructions] Rotate and Mask Word: logical right shift by (0 - RB) mod 64 per word; a count above 31 yields zero.
pub(super) fn rotm(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    words2(state, rt, ra, rb, |a, b| {
        shift_right_logical_word(a, negated_count(b, 0x3F))
    })
}

/// [SPU-ISA p:147 s:6. Shift and Rotate Instructions] Rotate and Mask Algebraic Word: arithmetic right shift by (0 - RB) mod 64 per word; a count above 31 fills with the sign bit.
pub(super) fn rotma(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    words2(state, rt, ra, rb, |a, b| {
        shift_right_algebraic_word(a, negated_count(b, 0x3F))
    })
}

/// [SPU-ISA p:118 s:6. Shift and Rotate Instructions] Shift Left Halfword: each halfword's count is bits 11 to 15 of its RB halfword; a count above 15 yields zero.
pub(super) fn shlh(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    halfwords2(state, rt, ra, rb, |a, b| {
        a.checked_shl(u32::from(b & 0x1F)).unwrap_or(0)
    })
}

/// [SPU-ISA p:119 s:6. Shift and Rotate Instructions] Shift Left Halfword Immediate: count is the low 5 bits of sign-extended I7; a count above 15 yields zero.
pub(super) fn shlhi(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    let s = u32::from(imm & 0x1F);
    halfwords2(state, rt, ra, ra, |a, _| a.checked_shl(s).unwrap_or(0))
}

/// [SPU-ISA p:127 s:6. Shift and Rotate Instructions] Rotate Halfword: each halfword's count is bits 12 to 15 of its RB halfword.
pub(super) fn roth(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    halfwords2(state, rt, ra, rb, |a, b| a.rotate_left(u32::from(b & 0x0F)))
}

/// [SPU-ISA p:128 s:6. Shift and Rotate Instructions] Rotate Halfword Immediate: count is the low 4 bits of I7.
pub(super) fn rothi(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    let s = u32::from(imm & 0x0F);
    halfwords2(state, rt, ra, ra, |a, _| a.rotate_left(s))
}

/// [SPU-ISA p:136 s:6. Shift and Rotate Instructions] Rotate and Mask Halfword: logical right shift by (0 - RB) mod 32 per halfword; a count above 15 yields zero.
pub(super) fn rothm(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    halfwords2(state, rt, ra, rb, |a, b| {
        a.checked_shr(negated_count(u32::from(b), 0x1F))
            .unwrap_or(0)
    })
}

/// [SPU-ISA p:137 s:6. Shift and Rotate Instructions] Rotate and Mask Halfword Immediate: logical right shift by (0 - I7) mod 32; a count above 15 yields zero.
pub(super) fn rothmi(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    let s = rotate_mask_count(imm) & 0x1F;
    halfwords2(state, rt, ra, ra, |a, _| a.checked_shr(s).unwrap_or(0))
}

/// [SPU-ISA p:145 s:6. Shift and Rotate Instructions] Rotate and Mask Algebraic Halfword: arithmetic right shift by (0 - RB) mod 32 per halfword; a count above 15 fills with the sign bit.
pub(super) fn rotmah(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    halfwords2(state, rt, ra, rb, |a, b| {
        shift_right_algebraic_halfword(a, negated_count(u32::from(b), 0x1F))
    })
}

/// [SPU-ISA p:146 s:6. Shift and Rotate Instructions] Rotate and Mask Algebraic Halfword Immediate: arithmetic right shift by (0 - I7) mod 32; a count above 15 fills with the sign bit.
pub(super) fn rotmahi(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    let s = rotate_mask_count(imm) & 0x1F;
    halfwords2(state, rt, ra, ra, |a, _| {
        shift_right_algebraic_halfword(a, s)
    })
}
