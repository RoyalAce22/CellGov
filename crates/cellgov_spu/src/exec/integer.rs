//! Integer arithmetic: word, halfword and byte add and subtract, the
//! multiplies, and the carry and borrow forms.

use super::lanes::{from_halfwords, from_words, halfwords, words};
use super::outcome::SpuStepOutcome;
use super::slots::{low_signed, words2};
use crate::state::SpuState;

/// The high halfword of a word, sign-extended.
fn high_signed(word: u32) -> i32 {
    i32::from((word >> 16) as u16 as i16)
}

/// [SPU-ISA p:60 s:5. Integer and Logical Instructions] Add Word: per-slot 32-bit modulo addition.
pub(super) fn a(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    for slot in 0..4 {
        let a = state.reg_word_slot(ra, slot);
        let b = state.reg_word_slot(rb, slot);
        state.set_reg_word_slot(rt, slot, a.wrapping_add(b));
    }
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:61 s:5. Integer and Logical Instructions] Add Word Immediate: per-slot 32-bit add of sign-extended I10.
pub(super) fn ai(state: &mut SpuState, rt: u8, ra: u8, imm: i16) -> SpuStepOutcome {
    let v = imm as i32 as u32;
    for slot in 0..4 {
        let a = state.reg_word_slot(ra, slot);
        state.set_reg_word_slot(rt, slot, a.wrapping_add(v));
    }
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:64 s:5. Integer and Logical Instructions] Subtract from Word: per-slot RB minus RA.
pub(super) fn sf(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    for slot in 0..4 {
        let a = state.reg_word_slot(ra, slot);
        let b = state.reg_word_slot(rb, slot);
        state.set_reg_word_slot(rt, slot, b.wrapping_sub(a));
    }
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:91 s:5. Integer and Logical Instructions] Average Bytes: (RA + RB + 1) >> 1 per unsigned byte, computed in nine bits.
pub(super) fn avgb(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let [a, b] = [ra, rb].map(|r| state.regs[r as usize]);
    state.set_reg(
        rt as usize,
        std::array::from_fn(|j| ((u16::from(a[j]) + u16::from(b[j]) + 1) >> 1) as u8),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:92 s:5. Integer and Logical Instructions] Absolute Differences of Bytes: |RB - RA| per unsigned byte.
pub(super) fn absdb(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let [a, b] = [ra, rb].map(|r| state.regs[r as usize]);
    state.set_reg(rt as usize, std::array::from_fn(|j| a[j].abs_diff(b[j])));
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:93 s:5. Integer and Logical Instructions] Sum Bytes into Halfwords: per word, RB's four-byte sum in the high halfword and RA's in the low.
pub(super) fn sumb(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let [a, b] = [ra, rb].map(|r| state.regs[r as usize]);
    let sum = |reg: [u8; 16], i: usize| -> u32 {
        reg[i * 4..i * 4 + 4]
            .iter()
            .map(|&byte| u32::from(byte))
            .sum()
    };
    state.set_reg(
        rt as usize,
        from_words(std::array::from_fn(|i| sum(b, i) << 16 | sum(a, i))),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:72 s:5. Integer and Logical Instructions] Multiply: signed low halfwords, 32-bit product.
pub(super) fn mpy(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    words2(state, rt, ra, rb, |a, b| {
        (low_signed(a) * low_signed(b)) as u32
    })
}

/// [SPU-ISA p:73 s:5. Integer and Logical Instructions] Multiply Unsigned: unsigned low halfwords.
pub(super) fn mpyu(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    words2(state, rt, ra, rb, |a, b| (a & 0xFFFF) * (b & 0xFFFF))
}

/// [SPU-ISA p:74 s:5. Integer and Logical Instructions] Multiply Immediate: I10 sign-extended to 16 bits times the signed low halfword.
pub(super) fn mpyi(state: &mut SpuState, rt: u8, ra: u8, imm: i16) -> SpuStepOutcome {
    words2(state, rt, ra, ra, |a, _| {
        (low_signed(a) * i32::from(imm)) as u32
    })
}

/// [SPU-ISA p:75 s:5. Integer and Logical Instructions] Multiply Unsigned Immediate: I10 extended to 16 bits, both operands unsigned.
pub(super) fn mpyui(state: &mut SpuState, rt: u8, ra: u8, imm: i16) -> SpuStepOutcome {
    words2(state, rt, ra, ra, |a, _| {
        (a & 0xFFFF) * u32::from(imm as u16)
    })
}

/// [SPU-ISA p:76 s:5. Integer and Logical Instructions] Multiply and Add: the signed low-halfword product plus RC.
pub(super) fn mpya(state: &mut SpuState, rt: u8, ra: u8, rb: u8, rc: u8) -> SpuStepOutcome {
    let [a, b, c] = [ra, rb, rc].map(|r| words(state.regs[r as usize]));
    state.set_reg(
        rt as usize,
        from_words(std::array::from_fn(|i| {
            ((low_signed(a[i]) * low_signed(b[i])) as u32).wrapping_add(c[i])
        })),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:77 s:5. Integer and Logical Instructions] Multiply High: the high halfword of RA times the low halfword of RB; the product's low 16 bits move to the high half.
pub(super) fn mpyh(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    words2(state, rt, ra, rb, |a, b| {
        (a >> 16).wrapping_mul(b & 0xFFFF) << 16
    })
}

/// [SPU-ISA p:78 s:5. Integer and Logical Instructions] Multiply and Shift Right: the product's high 16 bits, sign-extended.
pub(super) fn mpys(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    words2(state, rt, ra, rb, |a, b| {
        ((low_signed(a) * low_signed(b)) >> 16) as u32
    })
}

/// [SPU-ISA p:79 s:5. Integer and Logical Instructions] Multiply High High: signed high halfwords.
pub(super) fn mpyhh(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    words2(state, rt, ra, rb, |a, b| {
        (high_signed(a) * high_signed(b)) as u32
    })
}

/// [SPU-ISA p:80 s:5. Integer and Logical Instructions] Multiply High High and Add: the signed high-halfword product plus RT.
pub(super) fn mpyhha(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let [a, b, t] = [ra, rb, rt].map(|r| words(state.regs[r as usize]));
    state.set_reg(
        rt as usize,
        from_words(std::array::from_fn(|i| {
            ((high_signed(a[i]) * high_signed(b[i])) as u32).wrapping_add(t[i])
        })),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:81 s:5. Integer and Logical Instructions] Multiply High High Unsigned: unsigned high halfwords.
pub(super) fn mpyhhu(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    words2(state, rt, ra, rb, |a, b| (a >> 16) * (b >> 16))
}

/// [SPU-ISA p:82 s:5. Integer and Logical Instructions] Multiply High High Unsigned and Add: the unsigned high-halfword product plus RT.
pub(super) fn mpyhhau(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let [a, b, t] = [ra, rb, rt].map(|r| words(state.regs[r as usize]));
    state.set_reg(
        rt as usize,
        from_words(std::array::from_fn(|i| {
            ((a[i] >> 16) * (b[i] >> 16)).wrapping_add(t[i])
        })),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:66 s:5. Integer and Logical Instructions] Add Extended: RA + RB + the low bit of each RT word.
/// [SPU-ISA p:66 s:5] bits 0 to 30 of the RT input are reserved; the RTL reads bit 31 alone.
pub(super) fn addx(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let [a, b, t] = [ra, rb, rt].map(|r| words(state.regs[r as usize]));
    state.set_reg(
        rt as usize,
        from_words(std::array::from_fn(|i| {
            a[i].wrapping_add(b[i]).wrapping_add(t[i] & 1)
        })),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:67 s:5. Integer and Logical Instructions] Carry Generate: the carry out of RA + RB in bit 31, other bits zero.
pub(super) fn cg(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let [a, b] = [ra, rb].map(|r| words(state.regs[r as usize]));
    state.set_reg(
        rt as usize,
        from_words(std::array::from_fn(|i| {
            ((u64::from(a[i]) + u64::from(b[i])) >> 32) as u32
        })),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:68 s:5. Integer and Logical Instructions] Carry Generate Extended: the carry out of RA + RB + RT bit 31.
pub(super) fn cgx(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let [a, b, t] = [ra, rb, rt].map(|r| words(state.regs[r as usize]));
    state.set_reg(
        rt as usize,
        from_words(std::array::from_fn(|i| {
            ((u64::from(a[i]) + u64::from(b[i]) + u64::from(t[i] & 1)) >> 32) as u32
        })),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:69 s:5. Integer and Logical Instructions] Subtract from Extended: RB + not RA + RT bit 31.
pub(super) fn sfx(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let [a, b, t] = [ra, rb, rt].map(|r| words(state.regs[r as usize]));
    state.set_reg(
        rt as usize,
        from_words(std::array::from_fn(|i| {
            b[i].wrapping_add(!a[i]).wrapping_add(t[i] & 1)
        })),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:70 s:5. Integer and Logical Instructions] Borrow Generate: 1 when RB >= RA unsigned, else 0.
pub(super) fn bg(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let [a, b] = [ra, rb].map(|r| words(state.regs[r as usize]));
    state.set_reg(
        rt as usize,
        from_words(std::array::from_fn(|i| u32::from(b[i] >= a[i]))),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:71 s:5. Integer and Logical Instructions] Borrow Generate Extended: RB >= RA when RT bit 31 is set, RB > RA when it is clear.
pub(super) fn bgx(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let [a, b, t] = [ra, rb, rt].map(|r| words(state.regs[r as usize]));
    state.set_reg(
        rt as usize,
        from_words(std::array::from_fn(|i| {
            u32::from(if t[i] & 1 != 0 {
                b[i] >= a[i]
            } else {
                b[i] > a[i]
            })
        })),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:58 s:5. Integer and Logical Instructions] Add Halfword: per-halfword 16-bit modulo addition.
pub(super) fn ah(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let (a, b) = (
        halfwords(state.regs[ra as usize]),
        halfwords(state.regs[rb as usize]),
    );
    state.set_reg(
        rt as usize,
        from_halfwords(std::array::from_fn(|i| a[i].wrapping_add(b[i]))),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:59 s:5. Integer and Logical Instructions] Add Halfword Immediate: I10 sign-extended to 16 bits, added to each halfword.
pub(super) fn ahi(state: &mut SpuState, rt: u8, ra: u8, imm: i16) -> SpuStepOutcome {
    let a = halfwords(state.regs[ra as usize]);
    state.set_reg(
        rt as usize,
        from_halfwords(a.map(|h| h.wrapping_add(imm as u16))),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:62 s:5. Integer and Logical Instructions] Subtract from Halfword: per-halfword RB + not RA + 1.
pub(super) fn sfh(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let (a, b) = (
        halfwords(state.regs[ra as usize]),
        halfwords(state.regs[rb as usize]),
    );
    state.set_reg(
        rt as usize,
        from_halfwords(std::array::from_fn(|i| b[i].wrapping_sub(a[i]))),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:63 s:5. Integer and Logical Instructions] Subtract from Halfword Immediate: I10 sign-extended to 16 bits, minus each halfword.
pub(super) fn sfhi(state: &mut SpuState, rt: u8, ra: u8, imm: i16) -> SpuStepOutcome {
    let a = halfwords(state.regs[ra as usize]);
    state.set_reg(
        rt as usize,
        from_halfwords(a.map(|h| (imm as u16).wrapping_sub(h))),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:65 s:5. Integer and Logical Instructions] Subtract from Word Immediate: I10 sign-extended to 32 bits, minus each word.
pub(super) fn sfi(state: &mut SpuState, rt: u8, ra: u8, imm: i16) -> SpuStepOutcome {
    let a = words(state.regs[ra as usize]);
    state.set_reg(
        rt as usize,
        from_words(a.map(|w| (imm as i32 as u32).wrapping_sub(w))),
    );
    SpuStepOutcome::Continue
}
