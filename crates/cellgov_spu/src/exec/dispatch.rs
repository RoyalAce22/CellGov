//! The instruction dispatch: one match over every decoded SPU instruction.

use crate::instruction::SpuInstruction;
use crate::state::SpuState;
use crate::stop::SpuStopKind;
use cellgov_event::UnitId;

use super::channel::{execute_rchcnt, execute_rdch, execute_wrch};
use super::ls::{insertion_controls, load_quad, rotate_mask_count, store_quad, Lsa};
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

        // [SPU-ISA p:106 s:5. Integer and Logical Instructions] Or Word Immediate: per-slot OR of RA with sign-extended I10.
        SpuInstruction::Ori { rt, ra, imm } => {
            let v = imm as i32 as u32;
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                state.set_reg_word_slot(rt, slot, a | v);
            }
            SpuStepOutcome::Continue
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
            let shift = (imm & 0x1F) as usize;
            let src = state.regs[ra as usize];
            let mut dst = [0u8; 16];
            for i in 0..16 {
                dst[i] = if i + shift < 16 { src[i + shift] } else { 0 };
            }
            state.regs[rt as usize] = dst;
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:131 s:6. Shift and Rotate Instructions] Rotate Quadword by Bytes: byte rotate count taken from low nibble of RB preferred slot.
        SpuInstruction::Rotqby { rt, ra, rb } => {
            let shift = (state.reg_word(rb) & 0xF) as usize;
            let src = state.regs[ra as usize];
            let mut dst = [0u8; 16];
            for i in 0..16 {
                dst[i] = src[(i + shift) & 0xF];
            }
            state.regs[rt as usize] = dst;
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:132 s:6. Shift and Rotate Instructions] Rotate Quadword by Bytes Immediate: byte rotate count is the low nibble of I7.
        SpuInstruction::Rotqbyi { rt, ra, imm } => {
            let shift = (imm & 0xF) as usize;
            let src = state.regs[ra as usize];
            let mut dst = [0u8; 16];
            for i in 0..16 {
                dst[i] = src[(i + shift) & 0xF];
            }
            state.regs[rt as usize] = dst;
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:141 s:6. Shift and Rotate Instructions] Rotate and Mask Quadword by Bytes Immediate: shift right by (0 - I7) mod 32 bytes with zero fill; a count of 16 or more clears the register.
        SpuInstruction::Rotqmbyi { rt, ra, imm } => {
            let shift = (0u8.wrapping_sub(imm) & 0x1F) as usize;
            let src = state.regs[ra as usize];
            let mut dst = [0u8; 16];
            for (i, byte) in dst.iter_mut().enumerate() {
                *byte = if i >= shift { src[i - shift] } else { 0 };
            }
            state.regs[rt as usize] = dst;
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:120 s:6. Shift and Rotate Instructions] Shift Left Word: per-slot count is the low 6 bits of the RB slot; a count above 31 yields zero.
        SpuInstruction::Shl { rt, ra, rb } => {
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                let s = state.reg_word_slot(rb, slot) & 0x3F;
                state.set_reg_word_slot(rt, slot, a.checked_shl(s).unwrap_or(0));
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:121 s:6. Shift and Rotate Instructions] Shift Left Word Immediate: count is the low 6 bits of sign-extended I7; a count above 31 yields zero.
        SpuInstruction::Shli { rt, ra, imm } => {
            let s = (imm as u32) & 0x3F;
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                state.set_reg_word_slot(rt, slot, a.checked_shl(s).unwrap_or(0));
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:139 s:6. Shift and Rotate Instructions] Rotate and Mask Word Immediate: logical right shift by (0 - I7) mod 64; a count above 31 yields zero.
        SpuInstruction::Rotmi { rt, ra, imm } => {
            let s = rotate_mask_count(imm);
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                state.set_reg_word_slot(rt, slot, a.checked_shr(s).unwrap_or(0));
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:148 s:6. Shift and Rotate Instructions] Rotate and Mask Algebraic Word Immediate: arithmetic right shift by (0 - I7) mod 64; a count above 31 fills with the sign bit.
        SpuInstruction::Rotmai { rt, ra, imm } => {
            let s = rotate_mask_count(imm);
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot) as i32;
                let shifted = a.checked_shr(s).unwrap_or(a >> 31);
                state.set_reg_word_slot(rt, slot, shifted as u32);
            }
            SpuStepOutcome::Continue
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
