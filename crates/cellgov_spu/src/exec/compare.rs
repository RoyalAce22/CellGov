//! Integer compares: equal, greater than and logical greater than, by
//! byte, halfword and word.

use super::outcome::SpuStepOutcome;
use super::slots::{halfwords2, words2};
use crate::state::SpuState;

/// Applies `f` to each byte of `ra` and `rb` and writes the sixteen
/// results to `rt`, reading both sources before the write.
fn bytes2(
    state: &mut SpuState,
    rt: u8,
    ra: u8,
    rb: u8,
    f: impl Fn(u8, u8) -> u8,
) -> SpuStepOutcome {
    let [a, b] = [ra, rb].map(|r| state.regs[r as usize]);
    state.set_reg(rt as usize, std::array::from_fn(|i| f(a[i], b[i])));
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:160 s:7. Compare, Branch, and Halt Instructions] Compare Equal Word: per-slot all-ones if equal, else zero.
pub(super) fn ceq(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    for slot in 0..4 {
        let a = state.reg_word_slot(ra, slot);
        let b = state.reg_word_slot(rb, slot);
        state.set_reg_word_slot(rt, slot, if a == b { 0xFFFFFFFF } else { 0 });
    }
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:161 s:7. Compare, Branch, and Halt Instructions] Compare Equal Word Immediate: compare RA slot to sign-extended I10.
pub(super) fn ceqi(state: &mut SpuState, rt: u8, ra: u8, imm: i16) -> SpuStepOutcome {
    let v = imm as i32 as u32;
    for slot in 0..4 {
        let a = state.reg_word_slot(ra, slot);
        state.set_reg_word_slot(rt, slot, if a == v { 0xFFFFFFFF } else { 0 });
    }
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:157 s:7. Compare, Branch, and Halt Instructions] Compare Equal Byte Immediate: each byte of RA against the rightmost 8 bits of I10, all ones on a match.
pub(super) fn ceqbi(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    let a = state.regs[ra as usize];
    state.set_reg(
        rt as usize,
        a.map(|byte| if byte == imm { 0xFF } else { 0x00 }),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:167 s:7. Compare, Branch, and Halt Instructions] Compare Greater Than Word Immediate: signed compare of each RA slot against sign-extended I10.
pub(super) fn cgti(state: &mut SpuState, rt: u8, ra: u8, imm: i16) -> SpuStepOutcome {
    let v = imm as i32;
    for slot in 0..4 {
        let a = state.reg_word_slot(ra, slot) as i32;
        state.set_reg_word_slot(rt, slot, if a > v { 0xFFFFFFFF } else { 0 });
    }
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:172 s:7. Compare, Branch, and Halt Instructions] Compare Logical Greater Than Word: unsigned per-slot compare of RA against RB.
pub(super) fn clgt(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    for slot in 0..4 {
        let a = state.reg_word_slot(ra, slot);
        let b = state.reg_word_slot(rb, slot);
        state.set_reg_word_slot(rt, slot, if a > b { 0xFFFFFFFF } else { 0 });
    }
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:156 s:7. Compare, Branch, and Halt Instructions] Compare Equal Byte: per byte, all ones when RA equals RB.
pub(super) fn ceqb(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    bytes2(state, rt, ra, rb, |a, b| if a == b { 0xFF } else { 0 })
}

/// [SPU-ISA p:158 s:7. Compare, Branch, and Halt Instructions] Compare Equal Halfword: per halfword, all ones when RA equals RB.
pub(super) fn ceqh(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    halfwords2(state, rt, ra, rb, |a, b| if a == b { 0xFFFF } else { 0 })
}

/// [SPU-ISA p:159 s:7. Compare, Branch, and Halt Instructions] Compare Equal Halfword Immediate: per halfword against I10 sign-extended to 16 bits.
pub(super) fn ceqhi(state: &mut SpuState, rt: u8, ra: u8, imm: i16) -> SpuStepOutcome {
    halfwords2(
        state,
        rt,
        ra,
        ra,
        |a, _| if a == imm as u16 { 0xFFFF } else { 0 },
    )
}

/// [SPU-ISA p:162 s:7. Compare, Branch, and Halt Instructions] Compare Greater Than Byte: signed per byte.
pub(super) fn cgtb(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    bytes2(
        state,
        rt,
        ra,
        rb,
        |a, b| if a as i8 > b as i8 { 0xFF } else { 0 },
    )
}

/// [SPU-ISA p:163 s:7. Compare, Branch, and Halt Instructions] Compare Greater Than Byte Immediate: signed per byte against the rightmost 8 bits of I10.
pub(super) fn cgtbi(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    bytes2(state, rt, ra, ra, |a, _| {
        if a as i8 > imm as i8 {
            0xFF
        } else {
            0
        }
    })
}

/// [SPU-ISA p:164 s:7. Compare, Branch, and Halt Instructions] Compare Greater Than Halfword: signed per halfword.
pub(super) fn cgth(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    halfwords2(state, rt, ra, rb, |a, b| {
        if a as i16 > b as i16 {
            0xFFFF
        } else {
            0
        }
    })
}

/// [SPU-ISA p:165 s:7. Compare, Branch, and Halt Instructions] Compare Greater Than Halfword Immediate: signed per halfword against I10 sign-extended to 16 bits.
pub(super) fn cgthi(state: &mut SpuState, rt: u8, ra: u8, imm: i16) -> SpuStepOutcome {
    halfwords2(
        state,
        rt,
        ra,
        ra,
        |a, _| if a as i16 > imm { 0xFFFF } else { 0 },
    )
}

/// [SPU-ISA p:166 s:7. Compare, Branch, and Halt Instructions] Compare Greater Than Word: signed per word.
pub(super) fn cgt(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    words2(state, rt, ra, rb, |a, b| {
        if a as i32 > b as i32 {
            u32::MAX
        } else {
            0
        }
    })
}

/// [SPU-ISA p:168 s:7. Compare, Branch, and Halt Instructions] Compare Logical Greater Than Byte: unsigned per byte.
pub(super) fn clgtb(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    bytes2(state, rt, ra, rb, |a, b| if a > b { 0xFF } else { 0 })
}

/// [SPU-ISA p:169 s:7. Compare, Branch, and Halt Instructions] Compare Logical Greater Than Byte Immediate: unsigned per byte against the rightmost 8 bits of I10.
pub(super) fn clgtbi(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    bytes2(state, rt, ra, ra, |a, _| if a > imm { 0xFF } else { 0 })
}

/// [SPU-ISA p:170 s:7. Compare, Branch, and Halt Instructions] Compare Logical Greater Than Halfword: unsigned per halfword.
pub(super) fn clgth(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    halfwords2(state, rt, ra, rb, |a, b| if a > b { 0xFFFF } else { 0 })
}

/// [SPU-ISA p:171 s:7. Compare, Branch, and Halt Instructions] Compare Logical Greater Than Halfword Immediate: unsigned per halfword against I10 sign-extended to 16 bits.
pub(super) fn clgthi(state: &mut SpuState, rt: u8, ra: u8, imm: i16) -> SpuStepOutcome {
    halfwords2(
        state,
        rt,
        ra,
        ra,
        |a, _| if a > imm as u16 { 0xFFFF } else { 0 },
    )
}

/// [SPU-ISA p:173 s:7. Compare, Branch, and Halt Instructions] Compare Logical Greater Than Word Immediate: unsigned per word against I10 sign-extended to 32 bits.
pub(super) fn clgti(state: &mut SpuState, rt: u8, ra: u8, imm: i16) -> SpuStepOutcome {
    words2(state, rt, ra, ra, |a, _| {
        if a > imm as i32 as u32 {
            u32::MAX
        } else {
            0
        }
    })
}
