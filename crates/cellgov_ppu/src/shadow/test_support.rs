//! Shared raw-instruction encoders for shadow-pass tests. Compiled
//! only under `cfg(test)`. Tests of `mod`/`quicken`/`superpair` all
//! drive `PredecodedShadow::build` from a slice of u32 words; these
//! helpers produce the raw words.
//!
//! Defensive masking is applied to every operand field so an
//! out-of-range register or split-field index cannot silently
//! overflow into an adjacent field and produce a valid-looking
//! encoding for a different instruction. Debug asserts document
//! the architectural ranges; release builds keep the masks so a
//! release-mode test run that accidentally passes a too-wide value
//! still gets a deterministic encoding (the masked low bits) rather
//! than corrupted opcode bits.

use super::PredecodedShadow;

/// Assemble a shadow over `words` placed at `base`.
pub(super) fn build_from_words(base: u64, words: &[u32]) -> PredecodedShadow {
    let mut bytes = Vec::with_capacity(words.len() * 4);
    for &w in words {
        bytes.extend_from_slice(&w.to_be_bytes());
    }
    PredecodedShadow::build(base, &bytes)
}

/// Encode `addi rT, 0, simm` (i.e. `li rT, simm`).
pub(super) fn li_raw(rt: u32, simm: i16) -> u32 {
    debug_assert!(rt < 32, "GPR index out of range: {rt}");
    (14 << 26) | ((rt & 0x1F) << 21) | ((simm as u16) as u32)
}

/// Encode `b +offset` (unconditional, no link, no AA). The mask
/// `0x03FFFFFC` keeps only the LI field (PPC bits 6:29) and zeroes
/// AA/LK regardless of the caller's low bits.
pub(super) fn b_raw(offset: i32) -> u32 {
    (18 << 26) | ((offset as u32) & 0x03FFFFFC)
}

/// Encode `sc` (system call).
pub(super) fn sc_raw() -> u32 {
    (17 << 26) | 2
}

/// Encode `or rA, rS, rB` (opcode 31, XO 444).
pub(super) fn or_raw(rs: u32, ra: u32, rb: u32) -> u32 {
    debug_assert!(rs < 32 && ra < 32 && rb < 32, "GPR index out of range");
    (31 << 26) | ((rs & 0x1F) << 21) | ((ra & 0x1F) << 16) | ((rb & 0x1F) << 11) | (444 << 1)
}

/// Encode `rlwinm rA, rS, sh, mb, me` (opcode 21).
pub(super) fn rlwinm_raw(rs: u32, ra: u32, sh: u32, mb: u32, me: u32) -> u32 {
    debug_assert!(rs < 32 && ra < 32, "GPR index out of range");
    debug_assert!(sh < 32 && mb < 32 && me < 32, "rlwinm field out of range");
    (21 << 26)
        | ((rs & 0x1F) << 21)
        | ((ra & 0x1F) << 16)
        | ((sh & 0x1F) << 11)
        | ((mb & 0x1F) << 6)
        | ((me & 0x1F) << 1)
}

/// Encode `ori rA, rS, imm` (opcode 24).
pub(super) fn ori_raw(rs: u32, ra: u32, imm: u16) -> u32 {
    debug_assert!(rs < 32 && ra < 32, "GPR index out of range");
    (24 << 26) | ((rs & 0x1F) << 21) | ((ra & 0x1F) << 16) | (imm as u32)
}

/// Encode `rldicl rA, rS, sh, mb` (opcode 30, xo=0). `sh` and `mb`
/// are 6-bit split fields (range 0..64).
pub(super) fn rldicl_raw(rs: u32, ra: u32, sh: u32, mb: u32) -> u32 {
    debug_assert!(rs < 32 && ra < 32, "GPR index out of range");
    debug_assert!(sh < 64, "rldicl sh out of range: {sh}");
    debug_assert!(mb < 64, "rldicl mb out of range: {mb}");
    let sh = sh & 0x3F;
    let mb = mb & 0x3F;
    let sh_lo = sh & 0x1F;
    let sh_hi = (sh >> 5) & 1;
    let mb_lo = mb & 0x1F;
    let mb_hi = (mb >> 5) & 1;
    (30 << 26)
        | ((rs & 0x1F) << 21)
        | ((ra & 0x1F) << 16)
        | (sh_lo << 11)
        | (mb_lo << 6)
        | (mb_hi << 5)
        | (sh_hi << 1)
}

/// Encode `rldicr rA, rS, sh, me` (opcode 30, xo=1). `sh` and `me`
/// are 6-bit split fields (range 0..64).
pub(super) fn rldicr_raw(rs: u32, ra: u32, sh: u32, me: u32) -> u32 {
    debug_assert!(rs < 32 && ra < 32, "GPR index out of range");
    debug_assert!(sh < 64, "rldicr sh out of range: {sh}");
    debug_assert!(me < 64, "rldicr me out of range: {me}");
    let sh = sh & 0x3F;
    let me = me & 0x3F;
    let sh_lo = sh & 0x1F;
    let sh_hi = (sh >> 5) & 1;
    let me_lo = me & 0x1F;
    let me_hi = (me >> 5) & 1;
    (30 << 26)
        | ((rs & 0x1F) << 21)
        | ((ra & 0x1F) << 16)
        | (sh_lo << 11)
        | (me_lo << 6)
        | (me_hi << 5)
        | (1 << 2)
        | (sh_hi << 1)
}

/// Encode `lwz rT, off(rA)` (opcode 32).
pub(super) fn lwz_raw(rt: u32, ra: u32, off: i16) -> u32 {
    debug_assert!(rt < 32 && ra < 32, "GPR index out of range");
    (32 << 26) | ((rt & 0x1F) << 21) | ((ra & 0x1F) << 16) | (off as u16 as u32)
}

/// Encode `cmpwi crF, rA, imm` (opcode 11, L=0). `bf` is a 3-bit
/// CR field index; the L bit at u32 bit 21 stays zero by virtue of
/// no operand occupying that position.
pub(super) fn cmpwi_raw(bf: u32, ra: u32, imm: i16) -> u32 {
    debug_assert!(bf < 8, "CR field index out of range: {bf}");
    debug_assert!(ra < 32, "GPR index out of range");
    (11 << 26) | ((bf & 0x7) << 23) | ((ra & 0x1F) << 16) | (imm as u16 as u32)
}

/// Encode `cmpw crF, rA, rB` (opcode 31, XO 0, L=0).
pub(super) fn cmpw_raw(bf: u32, ra: u32, rb: u32) -> u32 {
    debug_assert!(bf < 8, "CR field index out of range: {bf}");
    debug_assert!(ra < 32 && rb < 32, "GPR index out of range");
    (31 << 26) | ((bf & 0x7) << 23) | ((ra & 0x1F) << 16) | ((rb & 0x1F) << 11)
}

/// Encode `stw rS, off(rA)` (opcode 36).
pub(super) fn stw_raw(rs: u32, ra: u32, off: i16) -> u32 {
    debug_assert!(rs < 32 && ra < 32, "GPR index out of range");
    (36 << 26) | ((rs & 0x1F) << 21) | ((ra & 0x1F) << 16) | (off as u16 as u32)
}

/// Encode `mflr rT` (opcode 31, XO=339, SPR=8).
pub(super) fn mflr_raw(rt: u32) -> u32 {
    debug_assert!(rt < 32, "GPR index out of range");
    (31 << 26) | ((rt & 0x1F) << 21) | (8 << 16) | (339 << 1)
}

/// Encode `mtlr rS` (opcode 31, XO=467, SPR=8).
pub(super) fn mtlr_raw(rs: u32) -> u32 {
    debug_assert!(rs < 32, "GPR index out of range");
    (31 << 26) | ((rs & 0x1F) << 21) | (8 << 16) | (467 << 1)
}

/// Encode `ld rT, off(rA)` (opcode 58, sub=0; off must be 4-aligned).
pub(super) fn ld_raw(rt: u32, ra: u32, off: i16) -> u32 {
    debug_assert!(rt < 32 && ra < 32, "GPR index out of range");
    (58 << 26) | ((rt & 0x1F) << 21) | ((ra & 0x1F) << 16) | ((off as u16 as u32) & 0xFFFC)
}

/// Encode `std rS, off(rA)` (opcode 62, sub=0; off must be 4-aligned).
pub(super) fn std_raw(rs: u32, ra: u32, off: i16) -> u32 {
    debug_assert!(rs < 32 && ra < 32, "GPR index out of range");
    (62 << 26) | ((rs & 0x1F) << 21) | ((ra & 0x1F) << 16) | ((off as u16 as u32) & 0xFFFC)
}

/// Encode `bc BO, BI, offset` (opcode 16, no link, no AA). The
/// `& 0xFFFC` mask keeps only the BD field (PPC bits 16:29) and
/// zeroes AA/LK regardless of what the caller passes; without it, a
/// non-4-aligned offset would silently flip AA or LK and produce a
/// different branch class.
pub(super) fn bc_raw(bo: u32, bi: u32, offset: i16) -> u32 {
    debug_assert!(bo < 32, "BO field out of range: {bo}");
    debug_assert!(bi < 32, "BI field out of range: {bi}");
    (16 << 26) | ((bo & 0x1F) << 21) | ((bi & 0x1F) << 16) | ((offset as u16 as u32) & 0xFFFC)
}
