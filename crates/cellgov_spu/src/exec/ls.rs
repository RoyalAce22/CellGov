//! Local-store addressing, the quadword load and store, and the shift and insertion helpers.

use crate::state::SpuState;

use super::outcome::{SpuFault, SpuStepOutcome};

/// The aligned local-store offset a quadword load or store resolves to.
///
/// The mask is the architecture's own, for a local store of
/// [`crate::state::SPU_LS_SIZE`]. It confines every address to local
/// store, so a guest cannot reach past the end however it computes the
/// address. The bound below therefore covers a `ls` shorter than the
/// architected size, which only a test builds. The fetch path faults on
/// a guest address; this path cannot.
// [SPU-ISA p:31 s:3. Memory-Load/Store Instructions] Every load/store address is first ANDed with the limit register, whose 256 KB value is 0x0003FFFF, and its low four bits are then dropped because only aligned quadwords move.
fn ls_addr(raw: u32, ls_len: usize) -> Result<usize, SpuFault> {
    let a = (raw & 0x3FFF0) as usize;
    if a + 16 > ls_len {
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
    match ls_addr(lsa.resolve(state), state.ls.len()) {
        Ok(a) => {
            state.regs[rt as usize].copy_from_slice(&state.ls[a..a + 16]);
            SpuStepOutcome::Continue
        }
        Err(f) => SpuStepOutcome::Fault(f),
    }
}

/// Copy `rt` to the aligned quadword at `lsa`. A fault leaves local
/// store unchanged.
#[inline]
pub(super) fn store_quad(state: &mut SpuState, rt: u8, lsa: Lsa) -> SpuStepOutcome {
    match ls_addr(lsa.resolve(state), state.ls.len()) {
        Ok(a) => {
            state.ls[a..a + 16].copy_from_slice(&state.regs[rt as usize]);
            SpuStepOutcome::Continue
        }
        Err(f) => SpuStepOutcome::Fault(f),
    }
}

// [SPU-ISA p:139 s:6. Shift and Rotate Instructions] The rotate-and-mask immediates carry the two's complement of the right-shift count: count = (0 - sign_extend(I7)) mod 64.
pub(super) fn rotate_mask_count(imm: u8) -> u32 {
    let signed = ((imm as u32) << 25) as i32 >> 25;
    (0i32.wrapping_sub(signed) as u32) & 0x3F
}

/// The shufb mask the generate-controls forms build.
///
/// Identity bytes fill `0x10..=0x1F`. The `width`-byte slot at `addr`
/// holds selectors for the rightmost `width` bytes of the preferred
/// slot, except a doubleword, which uses the leftmost 8.
// [SPU-ISA p:265 s:B. Details of the Generate Controls Instructions] The insertion mask shape per width.
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

#[cfg(test)]
#[path = "tests/exec_quad_tests.rs"]
mod quad_tests;
