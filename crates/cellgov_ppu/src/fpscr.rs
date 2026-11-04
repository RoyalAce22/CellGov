//! The FPSCR move instructions and the summary bits every FPSCR write recomputes.
//!
//! Each instruction takes its fields from the primary-63 decode: `frt`
//! is instruction bits 6:10, `fra` bits 11:15 and `frb` bits 16:20.

use cellgov_ps3_abi::hw::ppc_isa::{
    PPC_FPSCR_ENABLE_BITS, PPC_FPSCR_EXCEPTION_BITS, PPC_FPSCR_FEX, PPC_FPSCR_FX, PPC_FPSCR_VX,
    PPC_FPSCR_VX_BITS, PPE_FPSCR_UNIMPLEMENTED_BITS, PPE_MFFS_HIGH_WORD,
};

use crate::state::PpuState;

/// `fpscr` with VX and FEX recomputed from the bits they summarize.
///
/// [PPC-Book1 p:87 s:4.2.2] FEX and VX are the ORs of other FPSCR bits, so no instruction sets them explicitly.
///
/// It also clears [`PPE_FPSCR_UNIMPLEMENTED_BITS`], so no FPSCR write
/// keeps them.
pub(crate) fn with_summaries(fpscr: u32) -> u32 {
    let mut v = fpscr & !(PPC_FPSCR_VX | PPC_FPSCR_FEX | PPE_FPSCR_UNIMPLEMENTED_BITS);
    if v & PPC_FPSCR_VX_BITS != 0 {
        v |= PPC_FPSCR_VX;
    }
    // VX, OX, UX, ZX and XX (bits 2:6) sit 22 places above their
    // enables VE, OE, UE, ZE and XE (bits 24:28).
    if (v >> 22) & v & PPC_FPSCR_ENABLE_BITS != 0 {
        v |= PPC_FPSCR_FEX;
    }
    v
}

/// The mask of FPSCR field `field` (0..=7), field 0 in the high nibble.
fn field_mask(field: u8) -> u32 {
    0xF000_0000 >> (4 * u32::from(field))
}

/// The mask of FPSCR bit `bit` in PowerPC numbering (bit 0 is the MSB).
fn bit_mask(bit: u8) -> u32 {
    1 << (31 - u32::from(bit))
}

/// The CR1 nibble a record form copies: FX, FEX, VX and OX.
///
/// [PPC-Book1 p:18 s:2.3.1] For every floating-point instruction with Rc=1, CR1 is set from FPSCR bits 0:3 at completion.
pub(crate) fn cr1_nibble(fpscr: u32) -> u8 {
    (fpscr >> 28) as u8
}

/// `mffs`: `FRT[32:63]` <- FPSCR, and `FRT[0:31]` <- the PPE's high word.
///
/// [PPC-Book1 p:120 s:4.6.8] mffs places the FPSCR in FRT bits 32:63.
pub(crate) fn mffs(state: &mut PpuState, frt: u8) {
    let v = (u64::from(PPE_MFFS_HIGH_WORD) << 32) | u64::from(state.fpscr());
    state.set_fpr(frt as usize, v);
}

/// `mcrfs`: CR field BF <- FPSCR field BFA, then the exception bits
/// copied, and FX, are set to 0.
///
/// [PPC-Book1 p:120 s:4.6.8] mcrfs sets to 0 each exception bit it copies, and FX when it copies FX.
pub(crate) fn mcrfs(state: &mut PpuState, frt: u8, fra: u8) {
    let (bf, bfa) = (frt >> 2, fra >> 2);
    let fpscr = state.fpscr();
    let mask = field_mask(bfa);
    state.set_cr_field(bf, ((fpscr & mask) >> (28 - 4 * u32::from(bfa))) as u8);
    let cleared = mask & (PPC_FPSCR_EXCEPTION_BITS | PPC_FPSCR_FX);
    state.set_fpscr(with_summaries(fpscr & !cleared));
}

/// `mtfsfi`: FPSCR field BF <- U.
///
/// [PPC-Book1 p:121 s:4.6.8] mtfsfi takes FX from U when BF is 0; FEX and VX follow the usual rule.
pub(crate) fn mtfsfi(state: &mut PpuState, frt: u8, frb: u8) {
    let (bf, u) = (frt >> 2, u32::from(frb >> 1));
    let v = (state.fpscr() & !field_mask(bf)) | (u << (28 - 4 * u32::from(bf)));
    state.set_fpscr(with_summaries(v));
}

/// `mtfsf`: each FPSCR field whose FLM bit is set <- the same field of
/// `FRB[32:63]`.
///
/// [PPC-Book1 p:121 s:4.6.8] mtfsf sets FPSCR field i from the low-order word of FRB when FLM bit i is 1.
pub(crate) fn mtfsf(state: &mut PpuState, frt: u8, fra: u8, frb: u8) {
    // FLM is instruction bits 7:14: the low four bits of the `frt`
    // field and the high four of `fra`.
    let flm = ((frt & 0xF) << 4) | (fra >> 1);
    let mask = (0..8u8)
        .filter(|field| flm & (0x80 >> field) != 0)
        .fold(0, |m, field| m | field_mask(field));
    let src = state.fpr[frb as usize] as u32;
    let v = (state.fpscr() & !mask) | (src & mask);
    state.set_fpscr(with_summaries(v));
}

/// `mtfsb0`: FPSCR bit BT <- 0.
///
/// [PPC-Book1 p:122 s:4.6.8] mtfsb0 sets FPSCR bit BT to 0; it cannot reset FEX or VX.
pub(crate) fn mtfsb0(state: &mut PpuState, frt: u8) {
    state.set_fpscr(with_summaries(state.fpscr() & !bit_mask(frt)));
}

/// `mtfsb1`: FPSCR bit BT <- 1, and FX <- 1 when that turns an
/// exception bit from 0 to 1.
///
/// [PPC-Book1 p:122 s:4.6.8] mtfsb1 sets FPSCR bit BT to 1 and alters FX.
/// [PPC-Book1 p:87 s:4.2.2] Every floating-point instruction but mtfsfi and mtfsf sets FX when it turns an exception bit from 0 to 1.
pub(crate) fn mtfsb1(state: &mut PpuState, frt: u8) {
    let fpscr = state.fpscr();
    let bit = bit_mask(frt);
    let mut v = fpscr | bit;
    if bit & PPC_FPSCR_EXCEPTION_BITS & !fpscr != 0 {
        v |= PPC_FPSCR_FX;
    }
    state.set_fpscr(with_summaries(v));
}

#[cfg(test)]
#[path = "tests/fpscr_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/fpscr_console_tests.rs"]
mod console_tests;
