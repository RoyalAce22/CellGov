//! Constant formation: the immediate loads and the byte-mask immediate.

use super::outcome::SpuStepOutcome;
use crate::state::SpuState;

/// [SPU-ISA p:52 s:4. Constant-Formation Instructions] Immediate Load Word: replicate sign-extended I16 into all four word slots.
pub(super) fn il(state: &mut SpuState, rt: u8, imm: i16) -> SpuStepOutcome {
    state.set_reg_word_splat(rt, imm as i32 as u32);
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:53 s:4. Constant-Formation Instructions] Immediate Load Address: replicate I18 zero-extended into all word slots.
pub(super) fn ila(state: &mut SpuState, rt: u8, imm: u32) -> SpuStepOutcome {
    state.set_reg_word_splat(rt, imm);
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:50 s:4. Constant-Formation Instructions] Immediate Load Halfword: replicate I16 into each of the eight halfword slots.
pub(super) fn ilh(state: &mut SpuState, rt: u8, imm: u16) -> SpuStepOutcome {
    let hw = imm.to_be_bytes();
    state.set_reg(rt as usize, std::array::from_fn(|i| hw[i % 2]));
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:51 s:4. Constant-Formation Instructions] Immediate Load Halfword Upper: I16 placed in upper halfword of each word slot.
pub(super) fn ilhu(state: &mut SpuState, rt: u8, imm: u16) -> SpuStepOutcome {
    state.set_reg_word_splat(rt, (imm as u32) << 16);
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:54 s:4. Constant-Formation Instructions] Immediate Or Halfword Lower: OR I16 into the lower halfword of each word slot.
pub(super) fn iohl(state: &mut SpuState, rt: u8, imm: u16) -> SpuStepOutcome {
    for slot in 0..4 {
        let old = state.reg_word_slot(rt, slot);
        state.set_reg_word_slot(rt, slot, old | imm as u32);
    }
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:55 s:4. Constant-Formation Instructions] Form Select Mask for Bytes Immediate: each I16 bit expands to a 0x00 / 0xFF byte.
pub(super) fn fsmbi(state: &mut SpuState, rt: u8, imm: u16) -> SpuStepOutcome {
    let mut result = [0u8; 16];
    for (i, byte) in result.iter_mut().enumerate() {
        *byte = if (imm >> (15 - i)) & 1 != 0 {
            0xFF
        } else {
            0x00
        };
    }
    state.set_reg(rt as usize, result);
    SpuStepOutcome::Continue
}
