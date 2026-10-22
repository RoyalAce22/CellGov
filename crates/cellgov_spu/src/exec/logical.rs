//! Logical operations, sign extension, bit counts, select masks, bit
//! gathers and the byte shuffle.

use super::lanes::{from_halfwords, from_words, words};
use super::outcome::SpuStepOutcome;
use super::slots::{low_signed, words2};
use crate::state::SpuState;

/// [SPU-ISA p:116 s:5 Table 5-1] 10xxxxxx gives 0x00, 110xxxxx gives 0xFF, 111xxxxx gives 0x80.
fn shufb_byte(a: &[u8; 16], b: &[u8; 16], control: u8) -> u8 {
    match control {
        0x80..=0xBF => 0x00,
        0xC0..=0xDF => 0xFF,
        0xE0..=0xFF => 0x80,
        _ => {
            let index = usize::from(control & 0x1F);
            if index < 16 {
                a[index]
            } else {
                b[index - 16]
            }
        }
    }
}

/// All ones when bit `bit` of `word`, counted from the right, is set; zero otherwise.
fn mask_bit(word: u32, bit: usize) -> u32 {
    0u32.wrapping_sub((word >> bit) & 1)
}

/// `imm` in every byte of a word.
fn byte_mask(imm: u8) -> u32 {
    u32::from(imm) * 0x0101_0101
}

/// `imm`, sign-extended to 16 bits, in both halfwords of a word.
fn halfword_mask(imm: i16) -> u32 {
    u32::from(imm as u16) * 0x0001_0001
}

/// [SPU-ISA p:101 s:5. Integer and Logical Instructions] And Word Immediate: per-slot AND of RA with sign-extended I10.
pub(super) fn andi(state: &mut SpuState, rt: u8, ra: u8, imm: i16) -> SpuStepOutcome {
    let v = imm as i32 as u32;
    for slot in 0..4 {
        let a = state.reg_word_slot(ra, slot);
        state.set_reg_word_slot(rt, slot, a & v);
    }
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:106 s:5. Integer and Logical Instructions] Or Word Immediate: per-slot OR of RA with sign-extended I10.
pub(super) fn ori(state: &mut SpuState, rt: u8, ra: u8, imm: i16) -> SpuStepOutcome {
    let v = imm as i32 as u32;
    for slot in 0..4 {
        let a = state.reg_word_slot(ra, slot);
        state.set_reg_word_slot(rt, slot, a | v);
    }
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:98 s:5. Integer and Logical Instructions] And with Complement: RA AND the complement of RB.
pub(super) fn andc(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    words2(state, rt, ra, rb, |a, b| a & !b)
}

/// [SPU-ISA p:103 s:5. Integer and Logical Instructions] Or with Complement: RA OR the complement of RB.
pub(super) fn orc(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    words2(state, rt, ra, rb, |a, b| a | !b)
}

/// [SPU-ISA p:108 s:5. Integer and Logical Instructions] Exclusive Or: RA XOR RB.
pub(super) fn xor(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    words2(state, rt, ra, rb, |a, b| a ^ b)
}

/// [SPU-ISA p:112 s:5. Integer and Logical Instructions] Nand: the complement of RA AND RB.
pub(super) fn nand(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    words2(state, rt, ra, rb, |a, b| !(a & b))
}

/// [SPU-ISA p:114 s:5. Integer and Logical Instructions] Equivalent: RA XOR the complement of RB.
pub(super) fn eqv(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    words2(state, rt, ra, rb, |a, b| !(a ^ b))
}

/// [SPU-ISA p:107 s:5. Integer and Logical Instructions] Or Across: the OR of RA's four words in the preferred slot; the other slots are zero.
pub(super) fn orx(state: &mut SpuState, rt: u8, ra: u8) -> SpuStepOutcome {
    let w = words(state.regs[ra as usize]);
    state.set_reg(
        rt as usize,
        from_words([w[0] | w[1] | w[2] | w[3], 0, 0, 0]),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:99 s:5. Integer and Logical Instructions] And Byte Immediate: the rightmost 8 bits of I10, replicated into every byte.
pub(super) fn andbi(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    let mask = byte_mask(imm);
    words2(state, rt, ra, ra, |a, _| a & mask)
}

/// [SPU-ISA p:100 s:5. Integer and Logical Instructions] And Halfword Immediate: I10 sign-extended to 16 bits, replicated into every halfword.
pub(super) fn andhi(state: &mut SpuState, rt: u8, ra: u8, imm: i16) -> SpuStepOutcome {
    let mask = halfword_mask(imm);
    words2(state, rt, ra, ra, |a, _| a & mask)
}

/// [SPU-ISA p:104 s:5. Integer and Logical Instructions] Or Byte Immediate: the rightmost 8 bits of I10, replicated into every byte.
pub(super) fn orbi(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    let mask = byte_mask(imm);
    words2(state, rt, ra, ra, |a, _| a | mask)
}

/// [SPU-ISA p:105 s:5. Integer and Logical Instructions] Or Halfword Immediate: I10 sign-extended to 16 bits, replicated into every halfword.
pub(super) fn orhi(state: &mut SpuState, rt: u8, ra: u8, imm: i16) -> SpuStepOutcome {
    let mask = halfword_mask(imm);
    words2(state, rt, ra, ra, |a, _| a | mask)
}

/// [SPU-ISA p:109 s:5. Integer and Logical Instructions] Exclusive Or Byte Immediate: the rightmost 8 bits of I10, replicated into every byte.
pub(super) fn xorbi(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    let mask = byte_mask(imm);
    words2(state, rt, ra, ra, |a, _| a ^ mask)
}

/// [SPU-ISA p:110 s:5. Integer and Logical Instructions] Exclusive Or Halfword Immediate: I10 sign-extended to 16 bits, replicated into every halfword.
pub(super) fn xorhi(state: &mut SpuState, rt: u8, ra: u8, imm: i16) -> SpuStepOutcome {
    let mask = halfword_mask(imm);
    words2(state, rt, ra, ra, |a, _| a ^ mask)
}

/// [SPU-ISA p:111 s:5. Integer and Logical Instructions] Exclusive Or Word Immediate: I10 sign-extended to 32 bits.
pub(super) fn xori(state: &mut SpuState, rt: u8, ra: u8, imm: i16) -> SpuStepOutcome {
    let mask = imm as i32 as u32;
    words2(state, rt, ra, ra, |a, _| a ^ mask)
}

/// [SPU-ISA p:113 s:5. Integer and Logical Instructions] Nor: bitwise NOR across the full 128-bit register.
pub(super) fn nor(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let [a, b] = [ra, rb].map(|r| state.regs[r as usize]);
    state.set_reg(rt as usize, std::array::from_fn(|i| !(a[i] | b[i])));
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:97 s:5. Integer and Logical Instructions] And: bitwise AND across the full 128-bit register.
pub(super) fn and(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let [a, b] = [ra, rb].map(|r| state.regs[r as usize]);
    state.set_reg(rt as usize, std::array::from_fn(|i| a[i] & b[i]));
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:102 s:5. Integer and Logical Instructions] Or: bitwise OR across the full 128-bit register.
pub(super) fn or(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let [a, b] = [ra, rb].map(|r| state.regs[r as usize]);
    state.set_reg(rt as usize, std::array::from_fn(|i| a[i] | b[i]));
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:115 s:5. Integer and Logical Instructions] Select Bits: RC bits pick RB where set and RA where clear.
pub(super) fn selb(state: &mut SpuState, rt: u8, ra: u8, rb: u8, rc: u8) -> SpuStepOutcome {
    let [a, b, c] = [ra, rb, rc].map(|r| state.regs[r as usize]);
    state.set_reg(
        rt as usize,
        std::array::from_fn(|i| (c[i] & b[i]) | (!c[i] & a[i])),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:94 s:5. Integer and Logical Instructions] Extend Sign Byte to Halfword: each halfword takes the sign-extended value of its right byte.
pub(super) fn xsbh(state: &mut SpuState, rt: u8, ra: u8) -> SpuStepOutcome {
    let a = state.regs[ra as usize];
    state.set_reg(
        rt as usize,
        std::array::from_fn(|i| (a[i | 1] as i8 as i16).to_be_bytes()[i % 2]),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:95 s:5. Integer and Logical Instructions] Extend Sign Halfword to Word: each word takes the sign-extended value of its right halfword.
pub(super) fn xshw(state: &mut SpuState, rt: u8, ra: u8) -> SpuStepOutcome {
    words2(state, rt, ra, ra, |a, _| low_signed(a) as u32)
}

/// [SPU-ISA p:96 s:5. Integer and Logical Instructions] Extend Sign Word to Doubleword: each doubleword takes the sign-extended value of its right word.
pub(super) fn xswd(state: &mut SpuState, rt: u8, ra: u8) -> SpuStepOutcome {
    let w = words(state.regs[ra as usize]);
    let sign = |word: u32| ((word as i32) >> 31) as u32;
    state.set_reg(
        rt as usize,
        from_words([sign(w[1]), w[1], sign(w[3]), w[3]]),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:83 s:5. Integer and Logical Instructions] Count Leading Zeros: per word, 32 for a zero word.
pub(super) fn clz(state: &mut SpuState, rt: u8, ra: u8) -> SpuStepOutcome {
    words2(state, rt, ra, ra, |a, _| a.leading_zeros())
}

/// [SPU-ISA p:84 s:5. Integer and Logical Instructions] Count Ones in Bytes: the population count of each byte.
pub(super) fn cntb(state: &mut SpuState, rt: u8, ra: u8) -> SpuStepOutcome {
    state.set_reg(
        rt as usize,
        state.regs[ra as usize].map(|b| b.count_ones() as u8),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:85 s:5. Integer and Logical Instructions] Form Select Mask for Bytes: the preferred slot's rightmost 16 bits, leftmost bit to byte 0, each replicated eight times.
pub(super) fn fsmb(state: &mut SpuState, rt: u8, ra: u8) -> SpuStepOutcome {
    let s = state.reg_word_slot(ra, 0);
    state.set_reg(
        rt as usize,
        std::array::from_fn(|j| mask_bit(s, 15 - j) as u8),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:86 s:5. Integer and Logical Instructions] Form Select Mask for Halfwords: the preferred slot's rightmost 8 bits, leftmost bit to halfword 0, each replicated 16 times.
pub(super) fn fsmh(state: &mut SpuState, rt: u8, ra: u8) -> SpuStepOutcome {
    let s = state.reg_word_slot(ra, 0);
    state.set_reg(
        rt as usize,
        from_halfwords(std::array::from_fn(|j| mask_bit(s, 7 - j) as u16)),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:87 s:5. Integer and Logical Instructions] Form Select Mask for Words: the preferred slot's rightmost 4 bits, leftmost bit to word 0, each replicated 32 times.
pub(super) fn fsm(state: &mut SpuState, rt: u8, ra: u8) -> SpuStepOutcome {
    let s = state.reg_word_slot(ra, 0);
    state.set_reg(
        rt as usize,
        from_words(std::array::from_fn(|j| mask_bit(s, 3 - j))),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:88 s:5. Integer and Logical Instructions] Gather Bits from Bytes: the rightmost bit of each byte, byte 0 leftmost, forms the right half of the preferred slot; every other bit of RT is zero.
pub(super) fn gbb(state: &mut SpuState, rt: u8, ra: u8) -> SpuStepOutcome {
    let bits = state.regs[ra as usize]
        .iter()
        .fold(0u32, |bits, b| (bits << 1) | u32::from(b & 1));
    state.set_reg(rt as usize, [0u8; 16]);
    state.set_reg_word_slot(rt, 0, bits);
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:90 s:5. Integer and Logical Instructions] Gather Bits from Words: the low bit of each word, word 0 leftmost, forms a nibble in the preferred slot; every other bit of RT is zero.
pub(super) fn gb(state: &mut SpuState, rt: u8, ra: u8) -> SpuStepOutcome {
    let mut bits = 0u32;
    for slot in 0..4 {
        bits = (bits << 1) | (state.reg_word_slot(ra, slot) & 1);
    }
    state.set_reg(rt as usize, [0u8; 16]);
    state.set_reg_word_slot(rt, 0, bits);
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:89 s:5. Integer and Logical Instructions] Gather Bits from Halfwords: the low bit of each halfword, halfword 0 leftmost, forms a byte in the preferred slot; every other bit of RT is zero.
pub(super) fn gbh(state: &mut SpuState, rt: u8, ra: u8) -> SpuStepOutcome {
    let mut bits = 0u32;
    for slot in 0..8 {
        bits = (bits << 1) | (state.regs[ra as usize][slot * 2 + 1] & 1) as u32;
    }
    state.set_reg(rt as usize, [0u8; 16]);
    state.set_reg_word_slot(rt, 0, bits);
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:116 s:5. Integer and Logical Instructions] Shuffle Bytes: each RC byte selects a byte of RA||RB or a constant.
pub(super) fn shufb(state: &mut SpuState, rt: u8, ra: u8, rb: u8, rc: u8) -> SpuStepOutcome {
    let a = state.regs[ra as usize];
    let b = state.regs[rb as usize];
    let c = state.regs[rc as usize];
    state.set_reg(
        rt as usize,
        std::array::from_fn(|i| shufb_byte(&a, &b, c[i])),
    );
    SpuStepOutcome::Continue
}
