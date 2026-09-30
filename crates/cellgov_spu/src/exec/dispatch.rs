//! The instruction dispatch: one match over every decoded SPU instruction.

use crate::instruction::SpuInstruction;
use crate::state::SpuState;
use crate::stop::SpuStopKind;
use cellgov_event::UnitId;

use super::channel::{execute_rchcnt, execute_rdch, execute_wrch};
use super::lanes::{from_halfwords, from_words, halfwords, words};
use super::ls::{insertion_controls, load_quad, negated_count, rotate_mask_count, store_quad, Lsa};
use super::outcome::SpuStepOutcome;

/// The shared body of the indirect conditional branches.
fn branch_indirect_if(state: &mut SpuState, ra: u8, taken: bool) -> SpuStepOutcome {
    if taken {
        state.pc = state.insn_addr(state.reg_word(ra));
        SpuStepOutcome::Branch
    } else {
        SpuStepOutcome::Continue
    }
}

/// A halt instruction's outcome: a stop of kind `Halt` when its condition
/// holds, else the next instruction.
///
/// The ISA lets the SPU run zero or more instructions past a met halt;
/// CellGov runs none, so the resume address is the word after the halt.
// [SPU-ISA p:149 s:7] a halt stops imprecisely, at or after the halt instruction.
fn halt_if(condition: bool) -> SpuStepOutcome {
    if condition {
        SpuStepOutcome::Stop {
            kind: SpuStopKind::Halt,
            signal: 0,
        }
    } else {
        SpuStepOutcome::Continue
    }
}

// [SPU-ISA p:116 s:5 Table 5-1] 10xxxxxx gives 0x00, 110xxxxx gives 0xFF, 111xxxxx gives 0x80.
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

/// Applies `f` to each word slot of `ra` and `rb` and writes the four
/// results to `rt`.
///
/// It reads both sources before the write, so `rt` can alias `ra` or `rb`.
fn words2(
    state: &mut SpuState,
    rt: u8,
    ra: u8,
    rb: u8,
    f: impl Fn(u32, u32) -> u32,
) -> SpuStepOutcome {
    let [a, b] = [ra, rb].map(|r| words(state.regs[r as usize]));
    state.regs[rt as usize] = from_words(std::array::from_fn(|i| f(a[i], b[i])));
    SpuStepOutcome::Continue
}

/// The low halfword of a word, sign-extended.
fn low_signed(word: u32) -> i32 {
    i32::from(word as u16 as i16)
}

/// The high halfword of a word, sign-extended.
fn high_signed(word: u32) -> i32 {
    i32::from((word >> 16) as u16 as i16)
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

/// Applies `f` to each halfword slot of `ra` and `rb` and writes the eight
/// results to `rt`, reading both sources before the write.
fn halfwords2(
    state: &mut SpuState,
    rt: u8,
    ra: u8,
    rb: u8,
    f: impl Fn(u16, u16) -> u16,
) -> SpuStepOutcome {
    let [a, b] = [ra, rb].map(|r| halfwords(state.regs[r as usize]));
    state.regs[rt as usize] = from_halfwords(std::array::from_fn(|i| f(a[i], b[i])));
    SpuStepOutcome::Continue
}

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
    state.regs[rt as usize] = f(q).to_be_bytes();
    SpuStepOutcome::Continue
}

/// Execute a single decoded SPU instruction.
pub fn execute(insn: &SpuInstruction, state: &mut SpuState, unit_id: UnitId) -> SpuStepOutcome {
    match *insn {
        // [SPU-ISA p:32 s:3. Memory-Load/Store Instructions] Load Quadword (d-form): LSA from RA + I10<<4, force low 4 bits zero.
        SpuInstruction::Lqd { rt, ra, imm } => load_quad(state, rt, Lsa::D(ra, imm)),
        // [SPU-ISA p:33 s:3. Memory-Load/Store Instructions] Load Quadword (x-form): LSA from RA + RB, low 4 bits forced zero.
        SpuInstruction::Lqx { rt, ra, rb } => load_quad(state, rt, Lsa::X(ra, rb)),
        // [SPU-ISA p:34 s:3. Memory-Load/Store Instructions] Load Quadword (a-form): LSA is I16<<2, ignoring registers.
        SpuInstruction::Lqa { rt, imm } => load_quad(state, rt, Lsa::A(imm)),
        // [SPU-ISA p:35 s:3. Memory-Load/Store Instructions] Load Quadword Instruction Relative: LSA is PC + sign-extended I16<<2, low 4 bits forced zero.
        SpuInstruction::Lqr { rt, imm } => load_quad(state, rt, Lsa::R(imm)),
        // [SPU-ISA p:36 s:3. Memory-Load/Store Instructions] Store Quadword (d-form): symmetric to lqd, writes register to LS.
        SpuInstruction::Stqd { rt, ra, imm } => store_quad(state, rt, Lsa::D(ra, imm)),
        // [SPU-ISA p:37 s:3. Memory-Load/Store Instructions] Store Quadword (x-form): RA + RB indexed local-store address.
        SpuInstruction::Stqx { rt, ra, rb } => store_quad(state, rt, Lsa::X(ra, rb)),
        // [SPU-ISA p:38 s:3. Memory-Load/Store Instructions] Store Quadword (a-form): absolute LSA from I16<<2.
        SpuInstruction::Stqa { rt, imm } => store_quad(state, rt, Lsa::A(imm)),
        // [SPU-ISA p:39 s:3. Memory-Load/Store Instructions] Store Quadword Instruction Relative: PC-relative LSA, symmetric to lqr.
        SpuInstruction::Stqr { rt, imm } => store_quad(state, rt, Lsa::R(imm)),

        // [SPU-ISA p:52 s:4. Constant-Formation Instructions] Immediate Load Word: replicate sign-extended I16 into all four word slots.
        SpuInstruction::Il { rt, imm } => {
            state.set_reg_word_splat(rt, imm as i32 as u32);
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:53 s:4. Constant-Formation Instructions] Immediate Load Address: replicate I18 zero-extended into all word slots.
        SpuInstruction::Ila { rt, imm } => {
            state.set_reg_word_splat(rt, imm);
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:50 s:4. Constant-Formation Instructions] Immediate Load Halfword: replicate I16 into each of the eight halfword slots.
        SpuInstruction::Ilh { rt, imm } => {
            let hw = imm.to_be_bytes();
            let reg = &mut state.regs[rt as usize];
            for slot in 0..8 {
                reg[slot * 2] = hw[0];
                reg[slot * 2 + 1] = hw[1];
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:51 s:4. Constant-Formation Instructions] Immediate Load Halfword Upper: I16 placed in upper halfword of each word slot.
        SpuInstruction::Ilhu { rt, imm } => {
            state.set_reg_word_splat(rt, (imm as u32) << 16);
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:54 s:4. Constant-Formation Instructions] Immediate Or Halfword Lower: OR I16 into the lower halfword of each word slot.
        SpuInstruction::Iohl { rt, imm } => {
            for slot in 0..4 {
                let old = state.reg_word_slot(rt, slot);
                state.set_reg_word_slot(rt, slot, old | imm as u32);
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:55 s:4. Constant-Formation Instructions] Form Select Mask for Bytes Immediate: each I16 bit expands to a 0x00 / 0xFF byte.
        SpuInstruction::Fsmbi { rt, imm } => {
            let mut result = [0u8; 16];
            for (i, byte) in result.iter_mut().enumerate() {
                *byte = if (imm >> (15 - i)) & 1 != 0 {
                    0xFF
                } else {
                    0x00
                };
            }
            state.regs[rt as usize] = result;
            SpuStepOutcome::Continue
        }

        // [SPU-ISA p:60 s:5. Integer and Logical Instructions] Add Word: per-slot 32-bit modulo addition.
        SpuInstruction::A { rt, ra, rb } => {
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                let b = state.reg_word_slot(rb, slot);
                state.set_reg_word_slot(rt, slot, a.wrapping_add(b));
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:61 s:5. Integer and Logical Instructions] Add Word Immediate: per-slot 32-bit add of sign-extended I10.
        SpuInstruction::Ai { rt, ra, imm } => {
            let v = imm as i32 as u32;
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                state.set_reg_word_slot(rt, slot, a.wrapping_add(v));
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:101 s:5. Integer and Logical Instructions] And Word Immediate: per-slot AND of RA with sign-extended I10.
        SpuInstruction::Andi { rt, ra, imm } => {
            let v = imm as i32 as u32;
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                state.set_reg_word_slot(rt, slot, a & v);
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:64 s:5. Integer and Logical Instructions] Subtract from Word: per-slot RB minus RA.
        SpuInstruction::Sf { rt, ra, rb } => {
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                let b = state.reg_word_slot(rb, slot);
                state.set_reg_word_slot(rt, slot, b.wrapping_sub(a));
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:91 s:5. Integer and Logical Instructions] Average Bytes: (RA + RB + 1) >> 1 per unsigned byte, computed in nine bits.
        SpuInstruction::Avgb { rt, ra, rb } => {
            let [a, b] = [ra, rb].map(|r| state.regs[r as usize]);
            state.regs[rt as usize] =
                std::array::from_fn(|j| ((u16::from(a[j]) + u16::from(b[j]) + 1) >> 1) as u8);
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:92 s:5. Integer and Logical Instructions] Absolute Differences of Bytes: |RB - RA| per unsigned byte.
        SpuInstruction::Absdb { rt, ra, rb } => {
            let [a, b] = [ra, rb].map(|r| state.regs[r as usize]);
            state.regs[rt as usize] = std::array::from_fn(|j| a[j].abs_diff(b[j]));
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:93 s:5. Integer and Logical Instructions] Sum Bytes into Halfwords: per word, RB's four-byte sum in the high halfword and RA's in the low.
        SpuInstruction::Sumb { rt, ra, rb } => {
            let [a, b] = [ra, rb].map(|r| state.regs[r as usize]);
            let sum = |reg: [u8; 16], i: usize| -> u32 {
                reg[i * 4..i * 4 + 4]
                    .iter()
                    .map(|&byte| u32::from(byte))
                    .sum()
            };
            state.regs[rt as usize] =
                from_words(std::array::from_fn(|i| sum(b, i) << 16 | sum(a, i)));
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:72 s:5. Integer and Logical Instructions] Multiply: signed low halfwords, 32-bit product.
        SpuInstruction::Mpy { rt, ra, rb } => words2(state, rt, ra, rb, |a, b| {
            (low_signed(a) * low_signed(b)) as u32
        }),
        // [SPU-ISA p:73 s:5. Integer and Logical Instructions] Multiply Unsigned: unsigned low halfwords.
        SpuInstruction::Mpyu { rt, ra, rb } => {
            words2(state, rt, ra, rb, |a, b| (a & 0xFFFF) * (b & 0xFFFF))
        }
        // [SPU-ISA p:74 s:5. Integer and Logical Instructions] Multiply Immediate: I10 sign-extended to 16 bits times the signed low halfword.
        SpuInstruction::Mpyi { rt, ra, imm } => words2(state, rt, ra, ra, |a, _| {
            (low_signed(a) * i32::from(imm)) as u32
        }),
        // [SPU-ISA p:75 s:5. Integer and Logical Instructions] Multiply Unsigned Immediate: I10 extended to 16 bits, both operands unsigned.
        SpuInstruction::Mpyui { rt, ra, imm } => words2(state, rt, ra, ra, |a, _| {
            (a & 0xFFFF) * u32::from(imm as u16)
        }),
        // [SPU-ISA p:76 s:5. Integer and Logical Instructions] Multiply and Add: the signed low-halfword product plus RC.
        SpuInstruction::Mpya { rt, ra, rb, rc } => {
            let [a, b, c] = [ra, rb, rc].map(|r| words(state.regs[r as usize]));
            state.regs[rt as usize] = from_words(std::array::from_fn(|i| {
                ((low_signed(a[i]) * low_signed(b[i])) as u32).wrapping_add(c[i])
            }));
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:77 s:5. Integer and Logical Instructions] Multiply High: the high halfword of RA times the low halfword of RB; the product's low 16 bits move to the high half.
        SpuInstruction::Mpyh { rt, ra, rb } => words2(state, rt, ra, rb, |a, b| {
            (a >> 16).wrapping_mul(b & 0xFFFF) << 16
        }),
        // [SPU-ISA p:78 s:5. Integer and Logical Instructions] Multiply and Shift Right: the product's high 16 bits, sign-extended.
        SpuInstruction::Mpys { rt, ra, rb } => words2(state, rt, ra, rb, |a, b| {
            ((low_signed(a) * low_signed(b)) >> 16) as u32
        }),
        // [SPU-ISA p:79 s:5. Integer and Logical Instructions] Multiply High High: signed high halfwords.
        SpuInstruction::Mpyhh { rt, ra, rb } => words2(state, rt, ra, rb, |a, b| {
            (high_signed(a) * high_signed(b)) as u32
        }),
        // [SPU-ISA p:80 s:5. Integer and Logical Instructions] Multiply High High and Add: the signed high-halfword product plus RT.
        SpuInstruction::Mpyhha { rt, ra, rb } => {
            let [a, b, t] = [ra, rb, rt].map(|r| words(state.regs[r as usize]));
            state.regs[rt as usize] = from_words(std::array::from_fn(|i| {
                ((high_signed(a[i]) * high_signed(b[i])) as u32).wrapping_add(t[i])
            }));
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:81 s:5. Integer and Logical Instructions] Multiply High High Unsigned: unsigned high halfwords.
        SpuInstruction::Mpyhhu { rt, ra, rb } => {
            words2(state, rt, ra, rb, |a, b| (a >> 16) * (b >> 16))
        }
        // [SPU-ISA p:82 s:5. Integer and Logical Instructions] Multiply High High Unsigned and Add: the unsigned high-halfword product plus RT.
        SpuInstruction::Mpyhhau { rt, ra, rb } => {
            let [a, b, t] = [ra, rb, rt].map(|r| words(state.regs[r as usize]));
            state.regs[rt as usize] = from_words(std::array::from_fn(|i| {
                ((a[i] >> 16) * (b[i] >> 16)).wrapping_add(t[i])
            }));
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:66 s:5. Integer and Logical Instructions] Add Extended: RA + RB + the low bit of each RT word.
        // [SPU-ISA p:66 s:5] bits 0 to 30 of the RT input are reserved; the RTL reads bit 31 alone.
        SpuInstruction::Addx { rt, ra, rb } => {
            let [a, b, t] = [ra, rb, rt].map(|r| words(state.regs[r as usize]));
            state.regs[rt as usize] = from_words(std::array::from_fn(|i| {
                a[i].wrapping_add(b[i]).wrapping_add(t[i] & 1)
            }));
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:67 s:5. Integer and Logical Instructions] Carry Generate: the carry out of RA + RB in bit 31, other bits zero.
        SpuInstruction::Cg { rt, ra, rb } => {
            let [a, b] = [ra, rb].map(|r| words(state.regs[r as usize]));
            state.regs[rt as usize] = from_words(std::array::from_fn(|i| {
                ((u64::from(a[i]) + u64::from(b[i])) >> 32) as u32
            }));
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:68 s:5. Integer and Logical Instructions] Carry Generate Extended: the carry out of RA + RB + RT bit 31.
        SpuInstruction::Cgx { rt, ra, rb } => {
            let [a, b, t] = [ra, rb, rt].map(|r| words(state.regs[r as usize]));
            state.regs[rt as usize] = from_words(std::array::from_fn(|i| {
                ((u64::from(a[i]) + u64::from(b[i]) + u64::from(t[i] & 1)) >> 32) as u32
            }));
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:69 s:5. Integer and Logical Instructions] Subtract from Extended: RB + not RA + RT bit 31.
        SpuInstruction::Sfx { rt, ra, rb } => {
            let [a, b, t] = [ra, rb, rt].map(|r| words(state.regs[r as usize]));
            state.regs[rt as usize] = from_words(std::array::from_fn(|i| {
                b[i].wrapping_add(!a[i]).wrapping_add(t[i] & 1)
            }));
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:70 s:5. Integer and Logical Instructions] Borrow Generate: 1 when RB >= RA unsigned, else 0.
        SpuInstruction::Bg { rt, ra, rb } => {
            let [a, b] = [ra, rb].map(|r| words(state.regs[r as usize]));
            state.regs[rt as usize] = from_words(std::array::from_fn(|i| u32::from(b[i] >= a[i])));
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:71 s:5. Integer and Logical Instructions] Borrow Generate Extended: RB >= RA when RT bit 31 is set, RB > RA when it is clear.
        SpuInstruction::Bgx { rt, ra, rb } => {
            let [a, b, t] = [ra, rb, rt].map(|r| words(state.regs[r as usize]));
            state.regs[rt as usize] = from_words(std::array::from_fn(|i| {
                u32::from(if t[i] & 1 != 0 {
                    b[i] >= a[i]
                } else {
                    b[i] > a[i]
                })
            }));
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:58 s:5. Integer and Logical Instructions] Add Halfword: per-halfword 16-bit modulo addition.
        SpuInstruction::Ah { rt, ra, rb } => {
            let (a, b) = (
                halfwords(state.regs[ra as usize]),
                halfwords(state.regs[rb as usize]),
            );
            state.regs[rt as usize] =
                from_halfwords(std::array::from_fn(|i| a[i].wrapping_add(b[i])));
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:59 s:5. Integer and Logical Instructions] Add Halfword Immediate: I10 sign-extended to 16 bits, added to each halfword.
        SpuInstruction::Ahi { rt, ra, imm } => {
            let a = halfwords(state.regs[ra as usize]);
            state.regs[rt as usize] = from_halfwords(a.map(|h| h.wrapping_add(imm as u16)));
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:62 s:5. Integer and Logical Instructions] Subtract from Halfword: per-halfword RB + not RA + 1.
        SpuInstruction::Sfh { rt, ra, rb } => {
            let (a, b) = (
                halfwords(state.regs[ra as usize]),
                halfwords(state.regs[rb as usize]),
            );
            state.regs[rt as usize] =
                from_halfwords(std::array::from_fn(|i| b[i].wrapping_sub(a[i])));
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:63 s:5. Integer and Logical Instructions] Subtract from Halfword Immediate: I10 sign-extended to 16 bits, minus each halfword.
        SpuInstruction::Sfhi { rt, ra, imm } => {
            let a = halfwords(state.regs[ra as usize]);
            state.regs[rt as usize] = from_halfwords(a.map(|h| (imm as u16).wrapping_sub(h)));
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:65 s:5. Integer and Logical Instructions] Subtract from Word Immediate: I10 sign-extended to 32 bits, minus each word.
        SpuInstruction::Sfi { rt, ra, imm } => {
            let a = words(state.regs[ra as usize]);
            state.regs[rt as usize] = from_words(a.map(|w| (imm as i32 as u32).wrapping_sub(w)));
            SpuStepOutcome::Continue
        }

        // [SPU-ISA p:106 s:5. Integer and Logical Instructions] Or Word Immediate: per-slot OR of RA with sign-extended I10.
        SpuInstruction::Ori { rt, ra, imm } => {
            let v = imm as i32 as u32;
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                state.set_reg_word_slot(rt, slot, a | v);
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:98 s:5. Integer and Logical Instructions] And with Complement: RA AND the complement of RB.
        SpuInstruction::Andc { rt, ra, rb } => words2(state, rt, ra, rb, |a, b| a & !b),
        // [SPU-ISA p:103 s:5. Integer and Logical Instructions] Or with Complement: RA OR the complement of RB.
        SpuInstruction::Orc { rt, ra, rb } => words2(state, rt, ra, rb, |a, b| a | !b),
        // [SPU-ISA p:108 s:5. Integer and Logical Instructions] Exclusive Or: RA XOR RB.
        SpuInstruction::Xor { rt, ra, rb } => words2(state, rt, ra, rb, |a, b| a ^ b),
        // [SPU-ISA p:112 s:5. Integer and Logical Instructions] Nand: the complement of RA AND RB.
        SpuInstruction::Nand { rt, ra, rb } => words2(state, rt, ra, rb, |a, b| !(a & b)),
        // [SPU-ISA p:114 s:5. Integer and Logical Instructions] Equivalent: RA XOR the complement of RB.
        SpuInstruction::Eqv { rt, ra, rb } => words2(state, rt, ra, rb, |a, b| !(a ^ b)),
        // [SPU-ISA p:107 s:5. Integer and Logical Instructions] Or Across: the OR of RA's four words in the preferred slot; the other slots are zero.
        SpuInstruction::Orx { rt, ra } => {
            let w = words(state.regs[ra as usize]);
            state.regs[rt as usize] = from_words([w[0] | w[1] | w[2] | w[3], 0, 0, 0]);
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:99 s:5. Integer and Logical Instructions] And Byte Immediate: the rightmost 8 bits of I10, replicated into every byte.
        SpuInstruction::Andbi { rt, ra, imm } => {
            let mask = byte_mask(imm);
            words2(state, rt, ra, ra, |a, _| a & mask)
        }
        // [SPU-ISA p:100 s:5. Integer and Logical Instructions] And Halfword Immediate: I10 sign-extended to 16 bits, replicated into every halfword.
        SpuInstruction::Andhi { rt, ra, imm } => {
            let mask = halfword_mask(imm);
            words2(state, rt, ra, ra, |a, _| a & mask)
        }
        // [SPU-ISA p:104 s:5. Integer and Logical Instructions] Or Byte Immediate: the rightmost 8 bits of I10, replicated into every byte.
        SpuInstruction::Orbi { rt, ra, imm } => {
            let mask = byte_mask(imm);
            words2(state, rt, ra, ra, |a, _| a | mask)
        }
        // [SPU-ISA p:105 s:5. Integer and Logical Instructions] Or Halfword Immediate: I10 sign-extended to 16 bits, replicated into every halfword.
        SpuInstruction::Orhi { rt, ra, imm } => {
            let mask = halfword_mask(imm);
            words2(state, rt, ra, ra, |a, _| a | mask)
        }
        // [SPU-ISA p:109 s:5. Integer and Logical Instructions] Exclusive Or Byte Immediate: the rightmost 8 bits of I10, replicated into every byte.
        SpuInstruction::Xorbi { rt, ra, imm } => {
            let mask = byte_mask(imm);
            words2(state, rt, ra, ra, |a, _| a ^ mask)
        }
        // [SPU-ISA p:110 s:5. Integer and Logical Instructions] Exclusive Or Halfword Immediate: I10 sign-extended to 16 bits, replicated into every halfword.
        SpuInstruction::Xorhi { rt, ra, imm } => {
            let mask = halfword_mask(imm);
            words2(state, rt, ra, ra, |a, _| a ^ mask)
        }
        // [SPU-ISA p:111 s:5. Integer and Logical Instructions] Exclusive Or Word Immediate: I10 sign-extended to 32 bits.
        SpuInstruction::Xori { rt, ra, imm } => {
            let mask = imm as i32 as u32;
            words2(state, rt, ra, ra, |a, _| a ^ mask)
        }
        // [SPU-ISA p:113 s:5. Integer and Logical Instructions] Nor: bitwise NOR across the full 128-bit register.
        SpuInstruction::Nor { rt, ra, rb } => {
            for i in 0..16 {
                state.regs[rt as usize][i] =
                    !(state.regs[ra as usize][i] | state.regs[rb as usize][i]);
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:97 s:5. Integer and Logical Instructions] And: bitwise AND across the full 128-bit register.
        SpuInstruction::And { rt, ra, rb } => {
            for i in 0..16 {
                state.regs[rt as usize][i] =
                    state.regs[ra as usize][i] & state.regs[rb as usize][i];
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:102 s:5. Integer and Logical Instructions] Or: bitwise OR across the full 128-bit register.
        SpuInstruction::Or { rt, ra, rb } => {
            for i in 0..16 {
                state.regs[rt as usize][i] =
                    state.regs[ra as usize][i] | state.regs[rb as usize][i];
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:115 s:5. Integer and Logical Instructions] Select Bits: RC bits pick RB where set and RA where clear.
        SpuInstruction::Selb { rt, ra, rb, rc } => {
            for i in 0..16 {
                let c = state.regs[rc as usize][i];
                state.regs[rt as usize][i] =
                    (c & state.regs[rb as usize][i]) | (!c & state.regs[ra as usize][i]);
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:94 s:5. Integer and Logical Instructions] Extend Sign Byte to Halfword: each halfword takes the sign-extended value of its right byte.
        SpuInstruction::Xsbh { rt, ra } => {
            for slot in 0..8 {
                let low = state.regs[ra as usize][slot * 2 + 1];
                let hw = (low as i8 as i16).to_be_bytes();
                state.regs[rt as usize][slot * 2] = hw[0];
                state.regs[rt as usize][slot * 2 + 1] = hw[1];
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:95 s:5. Integer and Logical Instructions] Extend Sign Halfword to Word: each word takes the sign-extended value of its right halfword.
        SpuInstruction::Xshw { rt, ra } => words2(state, rt, ra, ra, |a, _| low_signed(a) as u32),
        // [SPU-ISA p:96 s:5. Integer and Logical Instructions] Extend Sign Word to Doubleword: each doubleword takes the sign-extended value of its right word.
        SpuInstruction::Xswd { rt, ra } => {
            let w = words(state.regs[ra as usize]);
            let sign = |word: u32| ((word as i32) >> 31) as u32;
            state.regs[rt as usize] = from_words([sign(w[1]), w[1], sign(w[3]), w[3]]);
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:83 s:5. Integer and Logical Instructions] Count Leading Zeros: per word, 32 for a zero word.
        SpuInstruction::Clz { rt, ra } => words2(state, rt, ra, ra, |a, _| a.leading_zeros()),
        // [SPU-ISA p:84 s:5. Integer and Logical Instructions] Count Ones in Bytes: the population count of each byte.
        SpuInstruction::Cntb { rt, ra } => {
            state.regs[rt as usize] = state.regs[ra as usize].map(|b| b.count_ones() as u8);
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:85 s:5. Integer and Logical Instructions] Form Select Mask for Bytes: the preferred slot's rightmost 16 bits, leftmost bit to byte 0, each replicated eight times.
        SpuInstruction::Fsmb { rt, ra } => {
            let s = state.reg_word_slot(ra, 0);
            state.regs[rt as usize] = std::array::from_fn(|j| mask_bit(s, 15 - j) as u8);
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:86 s:5. Integer and Logical Instructions] Form Select Mask for Halfwords: the preferred slot's rightmost 8 bits, leftmost bit to halfword 0, each replicated 16 times.
        SpuInstruction::Fsmh { rt, ra } => {
            let s = state.reg_word_slot(ra, 0);
            state.regs[rt as usize] =
                from_halfwords(std::array::from_fn(|j| mask_bit(s, 7 - j) as u16));
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:87 s:5. Integer and Logical Instructions] Form Select Mask for Words: the preferred slot's rightmost 4 bits, leftmost bit to word 0, each replicated 32 times.
        SpuInstruction::Fsm { rt, ra } => {
            let s = state.reg_word_slot(ra, 0);
            state.regs[rt as usize] = from_words(std::array::from_fn(|j| mask_bit(s, 3 - j)));
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:88 s:5. Integer and Logical Instructions] Gather Bits from Bytes: the rightmost bit of each byte, byte 0 leftmost, forms the right half of the preferred slot; every other bit of RT is zero.
        SpuInstruction::Gbb { rt, ra } => {
            let bits = state.regs[ra as usize]
                .iter()
                .fold(0u32, |bits, b| (bits << 1) | u32::from(b & 1));
            state.regs[rt as usize] = [0u8; 16];
            state.set_reg_word_slot(rt, 0, bits);
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:90 s:5. Integer and Logical Instructions] Gather Bits from Words: the low bit of each word, word 0 leftmost, forms a nibble in the preferred slot; every other bit of RT is zero.
        SpuInstruction::Gb { rt, ra } => {
            let mut bits = 0u32;
            for slot in 0..4 {
                bits = (bits << 1) | (state.reg_word_slot(ra, slot) & 1);
            }
            state.regs[rt as usize] = [0u8; 16];
            state.set_reg_word_slot(rt, 0, bits);
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:89 s:5. Integer and Logical Instructions] Gather Bits from Halfwords: the low bit of each halfword, halfword 0 leftmost, forms a byte in the preferred slot; every other bit of RT is zero.
        SpuInstruction::Gbh { rt, ra } => {
            let mut bits = 0u32;
            for slot in 0..8 {
                bits = (bits << 1) | (state.regs[ra as usize][slot * 2 + 1] & 1) as u32;
            }
            state.regs[rt as usize] = [0u8; 16];
            state.set_reg_word_slot(rt, 0, bits);
            SpuStepOutcome::Continue
        }

        // [SPU-ISA p:116 s:5. Integer and Logical Instructions] Shuffle Bytes: each RC byte selects a byte of RA||RB or a constant.
        SpuInstruction::Shufb { rt, ra, rb, rc } => {
            let a = state.regs[ra as usize];
            let b = state.regs[rb as usize];
            let c = state.regs[rc as usize];
            state.regs[rt as usize] = std::array::from_fn(|i| shufb_byte(&a, &b, c[i]));
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:125 s:6. Shift and Rotate Instructions] Shift Left Quadword by Bytes Immediate: shift register left by I7 bytes, fill zero.
        SpuInstruction::Shlqbyi { rt, ra, imm } => {
            let s = u32::from(imm & 0x1F);
            quad(state, rt, ra, |q| q.checked_shl(8 * s).unwrap_or(0))
        }
        // [SPU-ISA p:131 s:6. Shift and Rotate Instructions] Rotate Quadword by Bytes: byte rotate count taken from low nibble of RB preferred slot.
        SpuInstruction::Rotqby { rt, ra, rb } => {
            let s = state.reg_word_slot(rb, 0) & 0x0F;
            quad(state, rt, ra, |q| q.rotate_left(8 * s))
        }
        // [SPU-ISA p:132 s:6. Shift and Rotate Instructions] Rotate Quadword by Bytes Immediate: byte rotate count is the low nibble of I7.
        SpuInstruction::Rotqbyi { rt, ra, imm } => {
            let s = u32::from(imm & 0x0F);
            quad(state, rt, ra, |q| q.rotate_left(8 * s))
        }
        // [SPU-ISA p:141 s:6. Shift and Rotate Instructions] Rotate and Mask Quadword by Bytes Immediate: shift right by (0 - I7) mod 32 bytes with zero fill; a count of 16 or more clears the register.
        SpuInstruction::Rotqmbyi { rt, ra, imm } => {
            let s = negated_count(u32::from(imm), 0x1F);
            quad(state, rt, ra, |q| q.checked_shr(8 * s).unwrap_or(0))
        }
        // [SPU-ISA p:122 s:6. Shift and Rotate Instructions] Shift Left Quadword by Bits: count is bits 29 to 31 of RB's preferred slot.
        SpuInstruction::Shlqbi { rt, ra, rb } => {
            let s = state.reg_word_slot(rb, 0) & 0x07;
            quad(state, rt, ra, |q| q << s)
        }
        // [SPU-ISA p:123 s:6. Shift and Rotate Instructions] Shift Left Quadword by Bits Immediate: count is the low 3 bits of I7.
        SpuInstruction::Shlqbii { rt, ra, imm } => {
            let s = u32::from(imm & 0x07);
            quad(state, rt, ra, |q| q << s)
        }
        // [SPU-ISA p:134 s:6. Shift and Rotate Instructions] Rotate Quadword by Bits: count is bits 29 to 31 of RB's preferred slot.
        SpuInstruction::Rotqbi { rt, ra, rb } => {
            let s = state.reg_word_slot(rb, 0) & 0x07;
            quad(state, rt, ra, |q| q.rotate_left(s))
        }
        // [SPU-ISA p:135 s:6. Shift and Rotate Instructions] Rotate Quadword by Bits Immediate: count is the low 3 bits of I7.
        SpuInstruction::Rotqbii { rt, ra, imm } => {
            let s = u32::from(imm & 0x07);
            quad(state, rt, ra, |q| q.rotate_left(s))
        }
        // [SPU-ISA p:143 s:6. Shift and Rotate Instructions] Rotate and Mask Quadword by Bits: logical right shift by (0 - RB's preferred slot) mod 8.
        SpuInstruction::Rotqmbi { rt, ra, rb } => {
            let s = negated_count(state.reg_word_slot(rb, 0), 0x07);
            quad(state, rt, ra, |q| q >> s)
        }
        // [SPU-ISA p:144 s:6. Shift and Rotate Instructions] Rotate and Mask Quadword by Bits Immediate: logical right shift by (0 - I7) mod 8.
        SpuInstruction::Rotqmbii { rt, ra, imm } => {
            let s = negated_count(u32::from(imm), 0x07);
            quad(state, rt, ra, |q| q >> s)
        }
        // [SPU-ISA p:124 s:6. Shift and Rotate Instructions] Shift Left Quadword by Bytes: count is bits 27 to 31 of RB's preferred slot; a count above 15 yields zero.
        SpuInstruction::Shlqby { rt, ra, rb } => {
            let s = state.reg_word_slot(rb, 0) & 0x1F;
            quad(state, rt, ra, |q| q.checked_shl(8 * s).unwrap_or(0))
        }
        // [SPU-ISA p:126 s:6. Shift and Rotate Instructions] Shift Left Quadword by Bytes from Bit Shift Count: count is bits 24 to 28 of RB's preferred slot; a count above 15 yields zero.
        SpuInstruction::Shlqbybi { rt, ra, rb } => {
            let s = (state.reg_word_slot(rb, 0) >> 3) & 0x1F;
            quad(state, rt, ra, |q| q.checked_shl(8 * s).unwrap_or(0))
        }
        // [SPU-ISA p:133 s:6. Shift and Rotate Instructions] Rotate Quadword by Bytes from Bit Shift Count: the RTL reads bits 24 to 28 and rotates modulo 16, so bits 25 to 28 decide.
        SpuInstruction::Rotqbybi { rt, ra, rb } => {
            let s = (state.reg_word_slot(rb, 0) >> 3) & 0x0F;
            quad(state, rt, ra, |q| q.rotate_left(8 * s))
        }
        // [SPU-ISA p:140 s:6. Shift and Rotate Instructions] Rotate and Mask Quadword by Bytes: right shift by (0 - RB's preferred slot) mod 32 bytes; a count above 15 yields zero.
        SpuInstruction::Rotqmby { rt, ra, rb } => {
            let s = negated_count(state.reg_word_slot(rb, 0), 0x1F);
            quad(state, rt, ra, |q| q.checked_shr(8 * s).unwrap_or(0))
        }
        // [SPU-ISA p:142 s:6. Shift and Rotate Instructions] Rotate and Mask Quadword Bytes from Bit Shift Count: right shift by (0 - bits 24 to 28 of RB's preferred slot) mod 32 bytes.
        SpuInstruction::Rotqmbybi { rt, ra, rb } => {
            let s = negated_count(state.reg_word_slot(rb, 0) >> 3, 0x1F);
            quad(state, rt, ra, |q| q.checked_shr(8 * s).unwrap_or(0))
        }
        // [SPU-ISA p:120 s:6. Shift and Rotate Instructions] Shift Left Word: per-slot count is the low 6 bits of the RB slot; a count above 31 yields zero.
        SpuInstruction::Shl { rt, ra, rb } => words2(state, rt, ra, rb, |a, b| {
            a.checked_shl(b & 0x3F).unwrap_or(0)
        }),
        // [SPU-ISA p:121 s:6. Shift and Rotate Instructions] Shift Left Word Immediate: count is the low 6 bits of sign-extended I7; a count above 31 yields zero.
        SpuInstruction::Shli { rt, ra, imm } => {
            let s = u32::from(imm & 0x3F);
            words2(state, rt, ra, ra, |a, _| a.checked_shl(s).unwrap_or(0))
        }
        // [SPU-ISA p:139 s:6. Shift and Rotate Instructions] Rotate and Mask Word Immediate: logical right shift by (0 - I7) mod 64; a count above 31 yields zero.
        SpuInstruction::Rotmi { rt, ra, imm } => {
            let s = rotate_mask_count(imm);
            words2(state, rt, ra, ra, |a, _| shift_right_logical_word(a, s))
        }
        // [SPU-ISA p:148 s:6. Shift and Rotate Instructions] Rotate and Mask Algebraic Word Immediate: arithmetic right shift by (0 - I7) mod 64; a count above 31 fills with the sign bit.
        SpuInstruction::Rotmai { rt, ra, imm } => {
            let s = rotate_mask_count(imm);
            words2(state, rt, ra, ra, |a, _| shift_right_algebraic_word(a, s))
        }
        // [SPU-ISA p:129 s:6. Shift and Rotate Instructions] Rotate Word: each word's count is bits 27 to 31 of its RB word.
        SpuInstruction::Rot { rt, ra, rb } => {
            words2(state, rt, ra, rb, |a, b| a.rotate_left(b & 0x1F))
        }
        // [SPU-ISA p:130 s:6. Shift and Rotate Instructions] Rotate Word Immediate: count is the low 5 bits of sign-extended I7.
        SpuInstruction::Roti { rt, ra, imm } => {
            let s = u32::from(imm & 0x1F);
            words2(state, rt, ra, ra, |a, _| a.rotate_left(s))
        }
        // [SPU-ISA p:138 s:6. Shift and Rotate Instructions] Rotate and Mask Word: logical right shift by (0 - RB) mod 64 per word; a count above 31 yields zero.
        SpuInstruction::Rotm { rt, ra, rb } => words2(state, rt, ra, rb, |a, b| {
            shift_right_logical_word(a, negated_count(b, 0x3F))
        }),
        // [SPU-ISA p:147 s:6. Shift and Rotate Instructions] Rotate and Mask Algebraic Word: arithmetic right shift by (0 - RB) mod 64 per word; a count above 31 fills with the sign bit.
        SpuInstruction::Rotma { rt, ra, rb } => words2(state, rt, ra, rb, |a, b| {
            shift_right_algebraic_word(a, negated_count(b, 0x3F))
        }),

        // [SPU-ISA p:118 s:6. Shift and Rotate Instructions] Shift Left Halfword: each halfword's count is bits 11 to 15 of its RB halfword; a count above 15 yields zero.
        SpuInstruction::Shlh { rt, ra, rb } => halfwords2(state, rt, ra, rb, |a, b| {
            a.checked_shl(u32::from(b & 0x1F)).unwrap_or(0)
        }),
        // [SPU-ISA p:119 s:6. Shift and Rotate Instructions] Shift Left Halfword Immediate: count is the low 5 bits of sign-extended I7; a count above 15 yields zero.
        SpuInstruction::Shlhi { rt, ra, imm } => {
            let s = u32::from(imm & 0x1F);
            halfwords2(state, rt, ra, ra, |a, _| a.checked_shl(s).unwrap_or(0))
        }
        // [SPU-ISA p:127 s:6. Shift and Rotate Instructions] Rotate Halfword: each halfword's count is bits 12 to 15 of its RB halfword.
        SpuInstruction::Roth { rt, ra, rb } => {
            halfwords2(state, rt, ra, rb, |a, b| a.rotate_left(u32::from(b & 0x0F)))
        }
        // [SPU-ISA p:128 s:6. Shift and Rotate Instructions] Rotate Halfword Immediate: count is the low 4 bits of I7.
        SpuInstruction::Rothi { rt, ra, imm } => {
            let s = u32::from(imm & 0x0F);
            halfwords2(state, rt, ra, ra, |a, _| a.rotate_left(s))
        }
        // [SPU-ISA p:136 s:6. Shift and Rotate Instructions] Rotate and Mask Halfword: logical right shift by (0 - RB) mod 32 per halfword; a count above 15 yields zero.
        SpuInstruction::Rothm { rt, ra, rb } => halfwords2(state, rt, ra, rb, |a, b| {
            a.checked_shr(negated_count(u32::from(b), 0x1F))
                .unwrap_or(0)
        }),
        // [SPU-ISA p:137 s:6. Shift and Rotate Instructions] Rotate and Mask Halfword Immediate: logical right shift by (0 - I7) mod 32; a count above 15 yields zero.
        SpuInstruction::Rothmi { rt, ra, imm } => {
            let s = rotate_mask_count(imm) & 0x1F;
            halfwords2(state, rt, ra, ra, |a, _| a.checked_shr(s).unwrap_or(0))
        }
        // [SPU-ISA p:145 s:6. Shift and Rotate Instructions] Rotate and Mask Algebraic Halfword: arithmetic right shift by (0 - RB) mod 32 per halfword; a count above 15 fills with the sign bit.
        SpuInstruction::Rotmah { rt, ra, rb } => halfwords2(state, rt, ra, rb, |a, b| {
            shift_right_algebraic_halfword(a, negated_count(u32::from(b), 0x1F))
        }),
        // [SPU-ISA p:146 s:6. Shift and Rotate Instructions] Rotate and Mask Algebraic Halfword Immediate: arithmetic right shift by (0 - I7) mod 32; a count above 15 fills with the sign bit.
        SpuInstruction::Rotmahi { rt, ra, imm } => {
            let s = rotate_mask_count(imm) & 0x1F;
            halfwords2(state, rt, ra, ra, |a, _| {
                shift_right_algebraic_halfword(a, s)
            })
        }

        // [SPU-ISA p:40 s:3. Memory-Load/Store Instructions] Generate Controls for Byte Insertion (d-form): build shufb mask whose target byte position holds 0x03.
        SpuInstruction::Cbd { rt, ra, imm } => {
            state.regs[rt as usize] =
                insertion_controls(state.reg_word(ra).wrapping_add(imm as u32), 1);
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:41 s:3. Memory-Load/Store Instructions] Generate Controls for Byte Insertion (x-form): the byte position is RA + RB.
        SpuInstruction::Cbx { rt, ra, rb } => {
            state.regs[rt as usize] =
                insertion_controls(state.reg_word(ra).wrapping_add(state.reg_word(rb)), 1);
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:42 s:3. Memory-Load/Store Instructions] Generate Controls for Halfword Insertion (d-form): the aligned halfword slot holds 0x02 0x03.
        SpuInstruction::Chd { rt, ra, imm } => {
            state.regs[rt as usize] =
                insertion_controls(state.reg_word(ra).wrapping_add(imm as u32), 2);
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:43 s:3. Memory-Load/Store Instructions] Generate Controls for Halfword Insertion (x-form): the halfword position is RA + RB.
        SpuInstruction::Chx { rt, ra, rb } => {
            state.regs[rt as usize] =
                insertion_controls(state.reg_word(ra).wrapping_add(state.reg_word(rb)), 2);
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:44 s:3. Memory-Load/Store Instructions] Generate Controls for Word Insertion (d-form): build shufb mask placing 0x00..0x03 at the aligned word slot.
        SpuInstruction::Cwd { rt, ra, imm } => {
            state.regs[rt as usize] =
                insertion_controls(state.reg_word(ra).wrapping_add(imm as u32), 4);
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:45 s:3. Memory-Load/Store Instructions] Generate Controls for Word Insertion (x-form): the word position is RA + RB.
        SpuInstruction::Cwx { rt, ra, rb } => {
            state.regs[rt as usize] =
                insertion_controls(state.reg_word(ra).wrapping_add(state.reg_word(rb)), 4);
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:46 s:3. Memory-Load/Store Instructions] Generate Controls for Doubleword Insertion (d-form): the aligned doubleword slot holds 0x00..0x07.
        SpuInstruction::Cdd { rt, ra, imm } => {
            state.regs[rt as usize] =
                insertion_controls(state.reg_word(ra).wrapping_add(imm as u32), 8);
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:47 s:3. Memory-Load/Store Instructions] Generate Controls for Doubleword Insertion (x-form): the doubleword position is RA + RB.
        SpuInstruction::Cdx { rt, ra, rb } => {
            state.regs[rt as usize] =
                insertion_controls(state.reg_word(ra).wrapping_add(state.reg_word(rb)), 8);
            SpuStepOutcome::Continue
        }

        // [SPU-ISA p:160 s:7. Compare, Branch, and Halt Instructions] Compare Equal Word: per-slot all-ones if equal, else zero.
        SpuInstruction::Ceq { rt, ra, rb } => {
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                let b = state.reg_word_slot(rb, slot);
                state.set_reg_word_slot(rt, slot, if a == b { 0xFFFFFFFF } else { 0 });
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:161 s:7. Compare, Branch, and Halt Instructions] Compare Equal Word Immediate: compare RA slot to sign-extended I10.
        SpuInstruction::Ceqi { rt, ra, imm } => {
            let v = imm as i32 as u32;
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                state.set_reg_word_slot(rt, slot, if a == v { 0xFFFFFFFF } else { 0 });
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:157 s:7. Compare, Branch, and Halt Instructions] Compare Equal Byte Immediate: each byte of RA against the rightmost 8 bits of I10, all ones on a match.
        SpuInstruction::Ceqbi { rt, ra, imm } => {
            for i in 0..16 {
                let a = state.regs[ra as usize][i];
                state.regs[rt as usize][i] = if a == imm { 0xFF } else { 0x00 };
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:167 s:7. Compare, Branch, and Halt Instructions] Compare Greater Than Word Immediate: signed compare of each RA slot against sign-extended I10.
        SpuInstruction::Cgti { rt, ra, imm } => {
            let v = imm as i32;
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot) as i32;
                state.set_reg_word_slot(rt, slot, if a > v { 0xFFFFFFFF } else { 0 });
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:172 s:7. Compare, Branch, and Halt Instructions] Compare Logical Greater Than Word: unsigned per-slot compare of RA against RB.
        SpuInstruction::Clgt { rt, ra, rb } => {
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                let b = state.reg_word_slot(rb, slot);
                state.set_reg_word_slot(rt, slot, if a > b { 0xFFFFFFFF } else { 0 });
            }
            SpuStepOutcome::Continue
        }

        // [SPU-ISA p:174 s:7. Compare, Branch, and Halt Instructions] Branch Relative: PC <- PC + sign-extended I16<<2, masked to LS range.
        SpuInstruction::Br { offset } => {
            state.pc = state.insn_addr(state.pc.wrapping_add((offset << 2) as u32));
            SpuStepOutcome::Branch
        }
        // [SPU-ISA p:176 s:7. Compare, Branch, and Halt Instructions] Branch Relative and Set Link: the link is (PC+4) masked by LSLR in RT's preferred slot with the other slots zeroed, then the relative branch is taken.
        SpuInstruction::Brsl { rt, offset } => {
            let link = state.ls_wrap(state.pc.wrapping_add(4));
            state.regs[rt as usize] = [0u8; 16];
            state.set_reg_word_slot(rt, 0, link);
            state.pc = state.insn_addr(state.pc.wrapping_add((offset << 2) as u32));
            SpuStepOutcome::Branch
        }
        // [SPU-ISA p:183 s:7. Compare, Branch, and Halt Instructions] Branch If Zero Word: branch when RT preferred slot is zero.
        SpuInstruction::Brz { rt, offset } => {
            if state.reg_word(rt) == 0 {
                state.pc = state.insn_addr(state.pc.wrapping_add((offset << 2) as u32));
                SpuStepOutcome::Branch
            } else {
                SpuStepOutcome::Continue
            }
        }
        // [SPU-ISA p:182 s:7. Compare, Branch, and Halt Instructions] Branch If Not Zero Word: branch when RT preferred slot is non-zero.
        SpuInstruction::Brnz { rt, offset } => {
            if state.reg_word(rt) != 0 {
                state.pc = state.insn_addr(state.pc.wrapping_add((offset << 2) as u32));
                SpuStepOutcome::Branch
            } else {
                SpuStepOutcome::Continue
            }
        }
        // [SPU-ISA p:178 s:7. Compare, Branch, and Halt Instructions] Branch Indirect: PC <- RA preferred slot masked to LS range.
        SpuInstruction::Bi { ra } => {
            state.pc = state.insn_addr(state.reg_word(ra));
            SpuStepOutcome::Branch
        }
        // [SPU-ISA p:181 s:7. Compare, Branch, and Halt Instructions] Branch Indirect and Set Link: the target is read from RA before RT is written; the link is (PC+4) masked by LSLR in RT's preferred slot with the other slots zeroed, then PC <- RA masked to LS range.
        SpuInstruction::Bisl { rt, ra } => {
            let target = state.insn_addr(state.reg_word(ra));
            let link = state.ls_wrap(state.pc.wrapping_add(4));
            state.regs[rt as usize] = [0u8; 16];
            state.set_reg_word_slot(rt, 0, link);
            state.pc = target;
            SpuStepOutcome::Branch
        }
        // [SPU-ISA p:184 s:7. Compare, Branch, and Halt Instructions] Branch If Not Zero Halfword: branch when the low halfword of RT's preferred slot is non-zero.
        SpuInstruction::Brhnz { rt, offset } => {
            if state.reg_word(rt) & 0xFFFF != 0 {
                state.pc = state.insn_addr(state.pc.wrapping_add((offset << 2) as u32));
                SpuStepOutcome::Branch
            } else {
                SpuStepOutcome::Continue
            }
        }
        // [SPU-ISA p:185 s:7. Compare, Branch, and Halt Instructions] Branch If Zero Halfword: branch when the low halfword of RT's preferred slot is zero.
        SpuInstruction::Brhz { rt, offset } => {
            if state.reg_word(rt) & 0xFFFF == 0 {
                state.pc = state.insn_addr(state.pc.wrapping_add((offset << 2) as u32));
                SpuStepOutcome::Branch
            } else {
                SpuStepOutcome::Continue
            }
        }
        // [SPU-ISA p:186 s:7. Compare, Branch, and Halt Instructions] Branch Indirect If Zero: PC <- RA preferred slot masked to LS range when RT's preferred word is zero.
        SpuInstruction::Biz { rt, ra } => branch_indirect_if(state, ra, state.reg_word(rt) == 0),
        // [SPU-ISA p:187 s:7. Compare, Branch, and Halt Instructions] Branch Indirect If Not Zero: taken when RT's preferred word is non-zero.
        SpuInstruction::Binz { rt, ra } => branch_indirect_if(state, ra, state.reg_word(rt) != 0),
        // [SPU-ISA p:188 s:7. Compare, Branch, and Halt Instructions] Branch Indirect If Zero Halfword: taken when the low halfword of RT's preferred slot is zero.
        SpuInstruction::Bihz { rt, ra } => {
            branch_indirect_if(state, ra, state.reg_word(rt) & 0xFFFF == 0)
        }
        // [SPU-ISA p:189 s:7. Compare, Branch, and Halt Instructions] Branch Indirect If Not Zero Halfword: taken when the low halfword of RT's preferred slot is non-zero.
        SpuInstruction::Bihnz { rt, ra } => {
            branch_indirect_if(state, ra, state.reg_word(rt) & 0xFFFF != 0)
        }

        // [SPU-ISA p:250 s:11. Channel Instructions] Write Channel: send RT to the addressed channel.
        SpuInstruction::Wrch { channel, rt } => execute_wrch(channel, rt, state, unit_id),
        // [SPU-ISA p:248 s:11. Channel Instructions] Read Channel: capture channel value into RT, may stall on count.
        SpuInstruction::Rdch { rt, channel } => execute_rdch(rt, channel, state, unit_id),
        // [SPU-ISA p:249 s:11. Channel Instructions] Read Channel Count: the channel's capacity into RT's preferred slot, other slots zero.
        SpuInstruction::Rchcnt { rt, channel } => execute_rchcnt(rt, channel, state),

        // [SPU-ISA p:241 s:10. Control Instructions] No Operation (Execute) is architecturally a no-op.
        // [SPU-ISA p:240 s:10. Control Instructions] No Operation (Load) consumes only an even-pipe slot.
        // [SPU-ISA p:192 s:8. Hint-for-Branch Instructions] Hint for Branch (r-form) is a hint with no architectural effect.
        // [SPU-ISA p:194 s:8. Hint-for-Branch Instructions] Hint for Branch Relative is a hint with no architectural effect.
        // [SPU-ISA p:193 s:8. Hint-for-Branch Instructions] Hint for Branch (a-form) is a hint with no architectural effect.
        SpuInstruction::Nop
        | SpuInstruction::Lnop
        | SpuInstruction::Hbr
        | SpuInstruction::Hbra
        | SpuInstruction::Hbrr => SpuStepOutcome::Continue,

        // Every load, store and fetch here reads and writes local store in
        // program order, so each store is already visible to the next load
        // and the next fetch. Each channel write that `execute_wrch` accepts
        // takes effect before the next instruction, and the channels that
        // set execution state fall to its unsupported-channel fault. The
        // three barriers therefore order nothing further. This is one of
        // the outcomes the architecture allows.
        // [SPU-ISA p:242 s:10. Control Instructions] Synchronize waits for pending stores before the next fetch; the C bit first synchronizes channel state.
        // [SPU-ISA p:243 s:10. Control Instructions] Synchronize Data completes earlier loads, stores and channel accesses before later ones start.
        // [SPU-ISA p:254 s:13.1] local-store access is weakly consistent with respect to the instruction fetch.
        // [SPU-ISA p:255 s:13.3] without sync, the SPU might or might not execute a newly stored instruction.
        // [SPU-ISA p:256 s:13.5] an instruction the SPU fetched before the store is not seen, so self-modifying code runs a sync first.
        // [SPU-ISA p:258 s:13.9] only sync.c guarantees that a channel write to execution state affects the next instruction.
        SpuInstruction::Sync { c: _ } | SpuInstruction::Dsync => SpuStepOutcome::Continue,

        // [SPU-ISA p:244 s:10. Control Instructions] Move from SPR: an undefined SPR supplies zeros.
        // [CBE-Handbook p:67 s:3.1.2] the SPU has no special-purpose registers, so every SA reads zero.
        SpuInstruction::Mfspr { rt, sa: _ } => {
            state.regs[rt as usize] = [0u8; 16];
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:245 s:10. Control Instructions] Move to SPR: writing an undefined SPR performs no operation.
        // [CBE-Handbook p:67 s:3.1.2] the SPU has no special-purpose registers, so every write is dropped.
        SpuInstruction::Mtspr { sa: _, rt: _ } => SpuStepOutcome::Continue,

        // [SPU-ISA p:150 s:7. Compare, Branch, and Halt Instructions] Halt If Equal: stop when RA's preferred word equals RB's.
        SpuInstruction::Heq { ra, rb } => halt_if(state.reg_word(ra) == state.reg_word(rb)),
        // [SPU-ISA p:151 s:7. Compare, Branch, and Halt Instructions] Halt If Equal Immediate: I10 sign-extended to 32 bits.
        SpuInstruction::Heqi { ra, imm } => halt_if(state.reg_word(ra) == imm as i32 as u32),
        // [SPU-ISA p:152 s:7. Compare, Branch, and Halt Instructions] Halt If Greater Than: an algebraic compare.
        SpuInstruction::Hgt { ra, rb } => {
            halt_if(state.reg_word(ra) as i32 > state.reg_word(rb) as i32)
        }
        // [SPU-ISA p:153 s:7. Compare, Branch, and Halt Instructions] Halt If Greater Than Immediate: algebraic, against the sign-extended I10.
        SpuInstruction::Hgti { ra, imm } => halt_if(state.reg_word(ra) as i32 > i32::from(imm)),
        // [SPU-ISA p:154 s:7. Compare, Branch, and Halt Instructions] Halt If Logically Greater Than: an unsigned compare.
        SpuInstruction::Hlgt { ra, rb } => halt_if(state.reg_word(ra) > state.reg_word(rb)),
        // [SPU-ISA p:155 s:7. Compare, Branch, and Halt Instructions] Halt If Logically Greater Than Immediate: unsigned, against the sign-extended I10.
        SpuInstruction::Hlgti { ra, imm } => halt_if(state.reg_word(ra) > imm as i32 as u32),

        // [SPU-ISA p:238 s:10. Control Instructions] Stop and Signal halts the SPU and raises the stop signal to the PPE.
        // [SPU-ISA p:239 s:10. Control Instructions] Stop and Signal with Dependencies stops the SPU as stop does.
        SpuInstruction::Stop { signal } => SpuStepOutcome::Stop {
            kind: SpuStopKind::Stop,
            signal,
        },
        SpuInstruction::Stopd => SpuStepOutcome::Stop {
            kind: SpuStopKind::Stopd,
            signal: 0,
        },
    }
}

#[cfg(test)]
#[path = "tests/exec_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/exec_compiler_forms_tests.rs"]
mod compiler_forms_tests;

#[cfg(test)]
#[path = "tests/exec_job_forms_tests.rs"]
mod job_forms_tests;

#[cfg(test)]
#[path = "tests/lslr_tests.rs"]
mod lslr_tests;

#[cfg(test)]
#[path = "tests/halt_tests.rs"]
mod halt_tests;

#[cfg(test)]
#[path = "tests/spr_tests.rs"]
mod spr_tests;

#[cfg(test)]
#[path = "tests/sync_tests.rs"]
mod sync_tests;

#[cfg(test)]
#[path = "tests/channel_count_tests.rs"]
mod channel_count_tests;

#[cfg(test)]
#[path = "tests/start_state_tests.rs"]
mod start_state_tests;

#[cfg(test)]
#[path = "tests/shufb_tests.rs"]
mod shufb_tests;

#[cfg(test)]
#[path = "tests/reserved_channel_tests.rs"]
mod reserved_channel_tests;

#[cfg(test)]
#[path = "tests/channel_direction_tests.rs"]
mod channel_direction_tests;

#[cfg(test)]
#[path = "tests/tag_status_mask_tests.rs"]
mod tag_status_mask_tests;

#[cfg(test)]
#[path = "tests/tag_update_mode_tests.rs"]
mod tag_update_mode_tests;

#[cfg(test)]
#[path = "tests/halfword_arith_tests.rs"]
mod halfword_arith_tests;

#[cfg(test)]
#[path = "tests/carry_borrow_tests.rs"]
mod carry_borrow_tests;

#[cfg(test)]
#[path = "tests/multiply_tests.rs"]
mod multiply_tests;

#[cfg(test)]
#[path = "tests/bit_mask_tests.rs"]
mod bit_mask_tests;

#[cfg(test)]
#[path = "tests/byte_arith_tests.rs"]
mod byte_arith_tests;

#[cfg(test)]
#[path = "tests/sign_extend_tests.rs"]
mod sign_extend_tests;

#[cfg(test)]
#[path = "tests/logical_tests.rs"]
mod logical_tests;

#[cfg(test)]
#[path = "tests/halfword_shift_tests.rs"]
mod halfword_shift_tests;

#[cfg(test)]
#[path = "tests/word_rotate_tests.rs"]
mod word_rotate_tests;

#[cfg(test)]
#[path = "tests/quad_bit_shift_tests.rs"]
mod quad_bit_shift_tests;

#[cfg(test)]
#[path = "tests/quad_byte_shift_tests.rs"]
mod quad_byte_shift_tests;
