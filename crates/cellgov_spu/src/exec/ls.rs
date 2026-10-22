//! Local-store addressing, the quadword load and store, and the shift and insertion helpers.

use crate::state::SpuState;

use super::outcome::{SpuFault, SpuStepOutcome};

/// The aligned local-store offset a quadword load or store resolves to.
///
/// [`SpuState::quad_addr`] confines every address to the range the
/// limit register selects. A guest cannot reach past that range,
/// however it computes the address. The bound below covers a `ls`
/// shorter than the limit, which only a test builds.
///
/// [SPU-ISA p:31 s:3. Memory-Load/Store Instructions] Every load/store address is first ANDed with the limit register, whose 256 KB value is 0x0003FFFF, and its low four bits are then dropped because only aligned quadwords move.
fn ls_addr(state: &SpuState, raw: u32) -> Result<usize, SpuFault> {
    let a = state.quad_addr(raw) as usize;
    if a + 16 > state.ls.len() {
        Err(SpuFault::LsOutOfRange(raw))
    } else {
        Ok(a)
    }
}

/// The local-store address form of a quadword load or store.
#[derive(Clone, Copy)]
pub(super) enum Lsa {
    /// `RA + (I10 << 4)`.
    D(u8, i16),
    /// `RA + RB`.
    X(u8, u8),
    /// `I16 << 2`.
    A(i16),
    /// `PC + (I16 << 2)`.
    R(i16),
}

impl Lsa {
    #[inline]
    fn resolve(self, state: &SpuState) -> u32 {
        match self {
            Lsa::D(ra, imm) => state.reg_word(ra).wrapping_add((imm as i32 as u32) << 4),
            Lsa::X(ra, rb) => state.reg_word(ra).wrapping_add(state.reg_word(rb)),
            Lsa::A(imm) => (imm as i32 as u32) << 2,
            Lsa::R(imm) => state.pc.wrapping_add((imm as i32 as u32) << 2),
        }
    }
}

/// Copy the aligned quadword at `lsa` into `rt`. A fault leaves `rt`
/// unchanged.
#[inline]
pub(super) fn load_quad(state: &mut SpuState, rt: u8, lsa: Lsa) -> SpuStepOutcome {
    match ls_addr(state, lsa.resolve(state)) {
        Ok(a) => {
            let mut quad = [0u8; 16];
            quad.copy_from_slice(&state.ls[a..a + 16]);
            state.set_reg(rt as usize, quad);
            SpuStepOutcome::Continue
        }
        Err(f) => SpuStepOutcome::Fault(f),
    }
}

/// Copy `rt` to the aligned quadword at `lsa`. A fault leaves local
/// store unchanged.
#[inline]
pub(super) fn store_quad(state: &mut SpuState, rt: u8, lsa: Lsa) -> SpuStepOutcome {
    match ls_addr(state, lsa.resolve(state)) {
        Ok(a) => {
            state.ls[a..a + 16].copy_from_slice(&state.regs[rt as usize]);
            SpuStepOutcome::Continue
        }
        Err(f) => SpuStepOutcome::Fault(f),
    }
}

/// [SPU-ISA p:139 s:6. Shift and Rotate Instructions] The rotate-and-mask immediates carry the two's complement of the right-shift count: count = (0 - sign_extend(I7)) mod 64.
pub(super) fn rotate_mask_count(imm: u8) -> u32 {
    let signed = ((imm as u32) << 25) as i32 >> 25;
    negated_count(signed as u32, 0x3F)
}

/// [SPU-ISA p:138 s:6. Shift and Rotate Instructions] A rotate-and-mask count is the two's complement of its operand, reduced by the mask its form's RTL gives.
pub(super) fn negated_count(value: u32, mask: u32) -> u32 {
    0u32.wrapping_sub(value) & mask
}

/// The shufb mask the generate-controls forms build.
///
/// Identity bytes fill `0x10..=0x1F`. The `width`-byte slot at `addr`
/// holds selectors for the rightmost `width` bytes of the preferred
/// slot, except a doubleword, which uses the leftmost 8.
///
/// [SPU-ISA p:265 s:B. Details of the Generate Controls Instructions] The insertion mask shape per width.
pub(super) fn insertion_controls(addr: u32, width: usize) -> [u8; 16] {
    let pos = (addr as usize) & (0xF & !(width - 1));
    let first = if width == 8 { 0 } else { 4 - width as u8 };
    let mut mask = [0u8; 16];
    for (i, byte) in mask.iter_mut().enumerate() {
        *byte = if i >= pos && i < pos + width {
            first + (i - pos) as u8
        } else {
            0x10 + i as u8
        };
    }
    mask
}

/// [SPU-ISA p:32 s:3. Memory-Load/Store Instructions] Load Quadword (d-form): LSA from RA + I10<<4, force low 4 bits zero.
pub(super) fn lqd(state: &mut SpuState, rt: u8, ra: u8, imm: i16) -> SpuStepOutcome {
    load_quad(state, rt, Lsa::D(ra, imm))
}

/// [SPU-ISA p:33 s:3. Memory-Load/Store Instructions] Load Quadword (x-form): LSA from RA + RB, low 4 bits forced zero.
pub(super) fn lqx(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    load_quad(state, rt, Lsa::X(ra, rb))
}

/// [SPU-ISA p:34 s:3. Memory-Load/Store Instructions] Load Quadword (a-form): LSA is I16<<2, ignoring registers.
pub(super) fn lqa(state: &mut SpuState, rt: u8, imm: i16) -> SpuStepOutcome {
    load_quad(state, rt, Lsa::A(imm))
}

/// [SPU-ISA p:35 s:3. Memory-Load/Store Instructions] Load Quadword Instruction Relative: LSA is PC + sign-extended I16<<2, low 4 bits forced zero.
pub(super) fn lqr(state: &mut SpuState, rt: u8, imm: i16) -> SpuStepOutcome {
    load_quad(state, rt, Lsa::R(imm))
}

/// [SPU-ISA p:36 s:3. Memory-Load/Store Instructions] Store Quadword (d-form): symmetric to lqd, writes register to LS.
pub(super) fn stqd(state: &mut SpuState, rt: u8, ra: u8, imm: i16) -> SpuStepOutcome {
    store_quad(state, rt, Lsa::D(ra, imm))
}

/// [SPU-ISA p:37 s:3. Memory-Load/Store Instructions] Store Quadword (x-form): RA + RB indexed local-store address.
pub(super) fn stqx(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    store_quad(state, rt, Lsa::X(ra, rb))
}

/// [SPU-ISA p:38 s:3. Memory-Load/Store Instructions] Store Quadword (a-form): absolute LSA from I16<<2.
pub(super) fn stqa(state: &mut SpuState, rt: u8, imm: i16) -> SpuStepOutcome {
    store_quad(state, rt, Lsa::A(imm))
}

/// [SPU-ISA p:39 s:3. Memory-Load/Store Instructions] Store Quadword Instruction Relative: PC-relative LSA, symmetric to lqr.
pub(super) fn stqr(state: &mut SpuState, rt: u8, imm: i16) -> SpuStepOutcome {
    store_quad(state, rt, Lsa::R(imm))
}

/// [SPU-ISA p:40 s:3. Memory-Load/Store Instructions] Generate Controls for Byte Insertion (d-form): build shufb mask whose target byte position holds 0x03.
pub(super) fn cbd(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    state.set_reg(
        rt as usize,
        insertion_controls(state.reg_word(ra).wrapping_add(imm as u32), 1),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:41 s:3. Memory-Load/Store Instructions] Generate Controls for Byte Insertion (x-form): the byte position is RA + RB.
pub(super) fn cbx(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    state.set_reg(
        rt as usize,
        insertion_controls(state.reg_word(ra).wrapping_add(state.reg_word(rb)), 1),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:42 s:3. Memory-Load/Store Instructions] Generate Controls for Halfword Insertion (d-form): the aligned halfword slot holds 0x02 0x03.
pub(super) fn chd(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    state.set_reg(
        rt as usize,
        insertion_controls(state.reg_word(ra).wrapping_add(imm as u32), 2),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:43 s:3. Memory-Load/Store Instructions] Generate Controls for Halfword Insertion (x-form): the halfword position is RA + RB.
pub(super) fn chx(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    state.set_reg(
        rt as usize,
        insertion_controls(state.reg_word(ra).wrapping_add(state.reg_word(rb)), 2),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:44 s:3. Memory-Load/Store Instructions] Generate Controls for Word Insertion (d-form): build shufb mask placing 0x00..0x03 at the aligned word slot.
pub(super) fn cwd(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    state.set_reg(
        rt as usize,
        insertion_controls(state.reg_word(ra).wrapping_add(imm as u32), 4),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:45 s:3. Memory-Load/Store Instructions] Generate Controls for Word Insertion (x-form): the word position is RA + RB.
pub(super) fn cwx(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    state.set_reg(
        rt as usize,
        insertion_controls(state.reg_word(ra).wrapping_add(state.reg_word(rb)), 4),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:46 s:3. Memory-Load/Store Instructions] Generate Controls for Doubleword Insertion (d-form): the aligned doubleword slot holds 0x00..0x07.
pub(super) fn cdd(state: &mut SpuState, rt: u8, ra: u8, imm: u8) -> SpuStepOutcome {
    state.set_reg(
        rt as usize,
        insertion_controls(state.reg_word(ra).wrapping_add(imm as u32), 8),
    );
    SpuStepOutcome::Continue
}

/// [SPU-ISA p:47 s:3. Memory-Load/Store Instructions] Generate Controls for Doubleword Insertion (x-form): the doubleword position is RA + RB.
pub(super) fn cdx(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    state.set_reg(
        rt as usize,
        insertion_controls(state.reg_word(ra).wrapping_add(state.reg_word(rb)), 8),
    );
    SpuStepOutcome::Continue
}

#[cfg(test)]
#[path = "tests/exec_quad_tests.rs"]
mod quad_tests;
