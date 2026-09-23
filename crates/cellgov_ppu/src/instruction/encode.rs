//! Canonical 32-bit word for a decoded [`PpuInstruction`].
//!
//! [`encode`] is total over the variants the decoder produces, so
//! `decode(encode(i)) == Ok(i)` for every decoded `i`, and
//! `encode(decode(w))` is `w` with the bits in [`reserved_bits`]
//! cleared unless [`alias`] names `w` as a second spelling. A
//! variant with no arm here fails to compile.
// [PPC-Book1 p:7 s:1.7 Instruction formats] OPCD at bits 0:5; XO is form-dependent.

use cellgov_ps3_abi::hw::ppc_isa::{PPC_ISYNC_XO, PPC_STORAGE_HINT_XOS};

use super::ops::{VaOp, VxOp};
use super::{PpuInstruction, PpuInstructionKind};

/// Refusal to encode a variant that has no standalone 32-bit word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    /// The variant is a predecoded super-pair or its consumed slot.
    #[error("PPU instruction {kind:?} has no standalone encoding")]
    NoStandaloneEncoding {
        /// Variant that has no word of its own.
        kind: PpuInstructionKind,
    },
}

/// The second spelling of a decoded instruction that a raw word can carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alias {
    /// A barrier or cache hint the deterministic model decodes as `nop`.
    NopHint {
        /// Primary opcode of the hint.
        primary: u8,
        /// Extended opcode of the hint.
        xo: u16,
    },
    /// `mfspr` naming the time base, which `mftb` also reads.
    TimeBaseThroughMfspr {
        /// Time-base register number, 268 or 269.
        tbr: u16,
    },
}

const fn p(primary: u32) -> u32 {
    primary << 26
}
const fn rt(v: u8) -> u32 {
    ((v as u32) & 0x1F) << 21
}
const fn ra(v: u8) -> u32 {
    ((v as u32) & 0x1F) << 16
}
const fn rb(v: u8) -> u32 {
    ((v as u32) & 0x1F) << 11
}
const fn slot_6(v: u8) -> u32 {
    ((v as u32) & 0x1F) << 6
}
const fn xo_10(xo: u32, rc: bool) -> u32 {
    (xo << 1) | rc as u32
}
const fn xo_9(xo: u32, oe: bool, rc: bool) -> u32 {
    ((oe as u32) << 10) | (xo << 1) | rc as u32
}

// [PPC-Book1 p:8 s:1.7.4 D-Form] OPCD(0:5) RT/RS(6:10) RA(11:15) D/SI(16:31).
const fn d(primary: u32, t: u8, a: u8, imm: u16) -> u32 {
    p(primary) | rt(t) | ra(a) | imm as u32
}

// [PPC-Book1 p:8 s:1.7.5 DS-Form] DS(16:29) || 0b00; XO(30:31) selects the op.
const fn ds(primary: u32, t: u8, a: u8, imm: i16, sub: u32) -> u32 {
    p(primary) | rt(t) | ra(a) | ((imm as u16 as u32) & 0xFFFC) | sub
}

// [PPC-Book1 p:9 s:1.7.6 X-Form] OPCD(0:5) RT/RS(6:10) RA(11:15) RB(16:20) XO(21:30) Rc(31).
const fn x(t: u8, a: u8, b: u8, xo: u32, rc: bool) -> u32 {
    p(31) | rt(t) | ra(a) | rb(b) | xo_10(xo, rc)
}

// [PPC-Book1 p:9 s:1.7.11 XO-Form] OPCD RT RA RB OE(21) XO(22:30) Rc(31).
const fn xo(t: u8, a: u8, b: u8, xo: u32, oe: bool, rc: bool) -> u32 {
    p(31) | rt(t) | ra(a) | rb(b) | xo_9(xo, oe, rc)
}

// [PPC-Book1 p:9 s:1.7.8 XFX-Form] spr(11:20) is the SPR number with its halves swapped: the low five bits ride in the RA slot, the high five in the RB slot.
const fn xfx_spr(t: u8, spr: u16, xo: u32) -> u32 {
    let low = (spr as u32) & 0x1F;
    let high = ((spr as u32) >> 5) & 0x1F;
    p(31) | rt(t) | (low << 16) | (high << 11) | xo_10(xo, false)
}

// [PPC-Book1 p:9 s:1.7.7 XL-Form] OPCD BT/BO BA/BI BB(16:20) XO(21:30) LK(31).
const fn xl(t: u8, a: u8, b: u8, xo: u32, lk: bool) -> u32 {
    p(19) | rt(t) | ra(a) | rb(b) | xo_10(xo, lk)
}

// [PPC-Book1 p:10 s:1.7.13 M-Form] OPCD RS RA RB/SH MB(21:25) ME(26:30) Rc.
const fn m(primary: u32, s: u8, a: u8, third: u8, mb: u8, me: u8, rc: bool) -> u32 {
    p(primary) | rt(s) | ra(a) | rb(third) | slot_6(mb) | (((me as u32) & 0x1F) << 1) | rc as u32
}

// [PPC-Book1 p:10 s:1.7.14 MD-Form] OPCD RS RA sh(16:20) mb(21:25,26) XO(27:29) sh(30) Rc.
const fn md(s: u8, a: u8, sh: u8, mask: u8, xo: u32, rc: bool) -> u32 {
    let sh_hi = (((sh as u32) >> 5) & 1) << 1;
    let mask_hi = (((mask as u32) >> 5) & 1) << 5;
    p(30) | rt(s) | ra(a) | rb(sh) | slot_6(mask) | mask_hi | (xo << 2) | sh_hi | rc as u32
}

// [PPC-Book1 p:75 s:3.3.12] MDS-form: RB(16:20) mb/me(21:26) XO(27:30) Rc.
const fn mds(s: u8, a: u8, b: u8, mask: u8, xo: u32, rc: bool) -> u32 {
    let mask_hi = (((mask as u32) >> 5) & 1) << 5;
    p(30) | rt(s) | ra(a) | rb(b) | slot_6(mask) | mask_hi | (xo << 1) | rc as u32
}

// [PPC-Book1 p:10 s:1.7.12 A-Form] OPCD FRT FRA FRB FRC(21:25) XO(26:30) Rc(31).
const fn a_form(primary: u32, t: u8, a: u8, b: u8, c: u8, xo: u32, rc: bool) -> u32 {
    p(primary) | rt(t) | ra(a) | rb(b) | slot_6(c) | (xo << 1) | rc as u32
}

/// The canonical word for `insn`.
///
/// A quickened single instruction encodes as the base form its
/// extended mnemonic names, so the decoder returns that base variant
/// for the word.
///
/// # Errors
///
/// Returns [`EncodeError::NoStandaloneEncoding`] for a super-pair or
/// the consumed slot behind one.
pub fn encode(insn: &PpuInstruction) -> Result<u32, EncodeError> {
    use PpuInstruction as I;
    Ok(match *insn {
        // D-form loads.
        I::Lwz { rt: t, ra: a, imm } => d(32, t, a, imm as u16),
        I::Lwzu { rt: t, ra: a, imm } => d(33, t, a, imm as u16),
        I::Lbz { rt: t, ra: a, imm } => d(34, t, a, imm as u16),
        I::Lbzu { rt: t, ra: a, imm } => d(35, t, a, imm as u16),
        I::Lhz { rt: t, ra: a, imm } => d(40, t, a, imm as u16),
        I::Lhzu { rt: t, ra: a, imm } => d(41, t, a, imm as u16),
        I::Lha { rt: t, ra: a, imm } => d(42, t, a, imm as u16),
        I::Lhau { rt: t, ra: a, imm } => d(43, t, a, imm as u16),
        I::Lmw { rt: t, ra: a, imm } => d(46, t, a, imm as u16),
        I::Ld { rt: t, ra: a, imm } => ds(58, t, a, imm, 0),
        I::Ldu { rt: t, ra: a, imm } => ds(58, t, a, imm, 1),
        I::Lwa { rt: t, ra: a, imm } => ds(58, t, a, imm, 2),

        // D-form stores.
        I::Stw { rs, ra: a, imm } => d(36, rs, a, imm as u16),
        I::Stwu { rs, ra: a, imm } => d(37, rs, a, imm as u16),
        I::Stb { rs, ra: a, imm } => d(38, rs, a, imm as u16),
        I::Stbu { rs, ra: a, imm } => d(39, rs, a, imm as u16),
        I::Sth { rs, ra: a, imm } => d(44, rs, a, imm as u16),
        I::Sthu { rs, ra: a, imm } => d(45, rs, a, imm as u16),
        I::Stmw { rs, ra: a, imm } => d(47, rs, a, imm as u16),
        I::Std { rs, ra: a, imm } => ds(62, rs, a, imm, 0),
        I::Stdu { rs, ra: a, imm } => ds(62, rs, a, imm, 1),

        // D-form arithmetic and logical immediates.
        I::Mulli { rt: t, ra: a, imm } => d(7, t, a, imm as u16),
        I::Subfic { rt: t, ra: a, imm } => d(8, t, a, imm as u16),
        I::Addic { rt: t, ra: a, imm } => d(12, t, a, imm as u16),
        I::AddicDot { rt: t, ra: a, imm } => d(13, t, a, imm as u16),
        I::Addi { rt: t, ra: a, imm } => d(14, t, a, imm as u16),
        I::Addis { rt: t, ra: a, imm } => d(15, t, a, imm as u16),
        I::Ori { ra: a, rs, imm } => d(24, rs, a, imm),
        I::Oris { ra: a, rs, imm } => d(25, rs, a, imm),
        I::Xori { ra: a, rs, imm } => d(26, rs, a, imm),
        I::Xoris { ra: a, rs, imm } => d(27, rs, a, imm),
        I::AndiDot { ra: a, rs, imm } => d(28, rs, a, imm),
        I::AndisDot { ra: a, rs, imm } => d(29, rs, a, imm),

        // D-form compares: BF(6:8) /(9) L(10).
        I::Cmpwi { bf, ra: a, imm } => d(11, bf << 2, a, imm as u16),
        I::Cmpdi { bf, ra: a, imm } => d(11, (bf << 2) | 1, a, imm as u16),
        I::Cmplwi { bf, ra: a, imm } => d(10, bf << 2, a, imm),
        I::Cmpldi { bf, ra: a, imm } => d(10, (bf << 2) | 1, a, imm),

        // X-form compares: BF(6:8) /(9) L(10).
        I::Cmpw { bf, ra: a, rb: b } => x(bf << 2, a, b, 0, false),
        I::Cmpd { bf, ra: a, rb: b } => x((bf << 2) | 1, a, b, 0, false),
        I::Cmplw { bf, ra: a, rb: b } => x(bf << 2, a, b, 32, false),
        I::Cmpld { bf, ra: a, rb: b } => x((bf << 2) | 1, a, b, 32, false),

        // Branches.
        // [PPC-Book1 p:8 s:1.7.1 I-Form] LI(6:29) AA(30) LK(31).
        I::B { offset, aa, link } => {
            p(18) | ((offset as u32) & 0x03FF_FFFC) | ((aa as u32) << 1) | link as u32
        }
        // [PPC-Book1 p:8 s:1.7.2 B-Form] BO(6:10) BI(11:15) BD(16:29) AA(30) LK(31).
        I::Bc {
            bo,
            bi,
            offset,
            aa,
            link,
        } => {
            p(16)
                | rt(bo)
                | ra(bi)
                | ((offset as u16 as u32) & 0xFFFC)
                | ((aa as u32) << 1)
                | link as u32
        }
        I::Bclr { bo, bi, link } => xl(bo, bi, 0, 16, link),
        I::Bcctr { bo, bi, link } => xl(bo, bi, 0, 528, link),

        // XL-form condition-register operations.
        I::Mcrf { crfd, crfs } => xl(crfd << 2, crfs << 2, 0, 0, false),
        I::Crnor { bt, ba, bb } => xl(bt, ba, bb, 33, false),
        I::Crandc { bt, ba, bb } => xl(bt, ba, bb, 129, false),
        I::Crxor { bt, ba, bb } => xl(bt, ba, bb, 193, false),
        I::Crnand { bt, ba, bb } => xl(bt, ba, bb, 225, false),
        I::Crand { bt, ba, bb } => xl(bt, ba, bb, 257, false),
        I::Creqv { bt, ba, bb } => xl(bt, ba, bb, 289, false),
        I::Crorc { bt, ba, bb } => xl(bt, ba, bb, 417, false),
        I::Cror { bt, ba, bb } => xl(bt, ba, bb, 449, false),

        // XO-form arithmetic.
        I::Add {
            rt: t,
            ra: a,
            rb: b,
            oe,
            rc,
        } => xo(t, a, b, 266, oe, rc),
        I::Subf {
            rt: t,
            ra: a,
            rb: b,
            oe,
            rc,
        } => xo(t, a, b, 40, oe, rc),
        I::Subfc {
            rt: t,
            ra: a,
            rb: b,
            oe,
            rc,
        } => xo(t, a, b, 8, oe, rc),
        I::Subfe {
            rt: t,
            ra: a,
            rb: b,
            oe,
            rc,
        } => xo(t, a, b, 136, oe, rc),
        I::Neg {
            rt: t,
            ra: a,
            oe,
            rc,
        } => xo(t, a, 0, 104, oe, rc),
        I::Mullw {
            rt: t,
            ra: a,
            rb: b,
            oe,
            rc,
        } => xo(t, a, b, 235, oe, rc),
        I::Mulhwu {
            rt: t,
            ra: a,
            rb: b,
            rc,
        } => xo(t, a, b, 11, false, rc),
        I::Mulhdu {
            rt: t,
            ra: a,
            rb: b,
            rc,
        } => xo(t, a, b, 9, false, rc),
        I::Mulhd {
            rt: t,
            ra: a,
            rb: b,
            rc,
        } => xo(t, a, b, 73, false, rc),
        I::Mulhw {
            rt: t,
            ra: a,
            rb: b,
            rc,
        } => xo(t, a, b, 75, false, rc),
        I::Adde {
            rt: t,
            ra: a,
            rb: b,
            oe,
            rc,
        } => xo(t, a, b, 138, oe, rc),
        I::Addze {
            rt: t,
            ra: a,
            oe,
            rc,
        } => xo(t, a, 0, 202, oe, rc),
        I::Subfze {
            rt: t,
            ra: a,
            oe,
            rc,
        } => xo(t, a, 0, 200, oe, rc),
        I::Subfme {
            rt: t,
            ra: a,
            oe,
            rc,
        } => xo(t, a, 0, 232, oe, rc),
        I::Addme {
            rt: t,
            ra: a,
            oe,
            rc,
        } => xo(t, a, 0, 234, oe, rc),
        I::Divw {
            rt: t,
            ra: a,
            rb: b,
            oe,
            rc,
        } => xo(t, a, b, 491, oe, rc),
        I::Divwu {
            rt: t,
            ra: a,
            rb: b,
            oe,
            rc,
        } => xo(t, a, b, 459, oe, rc),
        I::Divd {
            rt: t,
            ra: a,
            rb: b,
            oe,
            rc,
        } => xo(t, a, b, 489, oe, rc),
        I::Divdu {
            rt: t,
            ra: a,
            rb: b,
            oe,
            rc,
        } => xo(t, a, b, 457, oe, rc),
        I::Mulld {
            rt: t,
            ra: a,
            rb: b,
            oe,
            rc,
        } => xo(t, a, b, 233, oe, rc),

        // X-form logical, shift, and extend; RS rides in the RT slot.
        I::Or {
            ra: a,
            rs,
            rb: b,
            rc,
        } => x(rs, a, b, 444, rc),
        I::Orc {
            ra: a,
            rs,
            rb: b,
            rc,
        } => x(rs, a, b, 412, rc),
        I::And {
            ra: a,
            rs,
            rb: b,
            rc,
        } => x(rs, a, b, 28, rc),
        I::Andc {
            ra: a,
            rs,
            rb: b,
            rc,
        } => x(rs, a, b, 60, rc),
        I::Nor {
            ra: a,
            rs,
            rb: b,
            rc,
        } => x(rs, a, b, 124, rc),
        I::Xor {
            ra: a,
            rs,
            rb: b,
            rc,
        } => x(rs, a, b, 316, rc),
        I::Eqv {
            ra: a,
            rs,
            rb: b,
            rc,
        } => x(rs, a, b, 284, rc),
        I::Nand {
            ra: a,
            rs,
            rb: b,
            rc,
        } => x(rs, a, b, 476, rc),
        I::Slw {
            ra: a,
            rs,
            rb: b,
            rc,
        } => x(rs, a, b, 24, rc),
        I::Srw {
            ra: a,
            rs,
            rb: b,
            rc,
        } => x(rs, a, b, 536, rc),
        I::Sld {
            ra: a,
            rs,
            rb: b,
            rc,
        } => x(rs, a, b, 27, rc),
        I::Srd {
            ra: a,
            rs,
            rb: b,
            rc,
        } => x(rs, a, b, 539, rc),
        I::Sraw {
            ra: a,
            rs,
            rb: b,
            rc,
        } => x(rs, a, b, 792, rc),
        I::Srad {
            ra: a,
            rs,
            rb: b,
            rc,
        } => x(rs, a, b, 794, rc),
        I::Srawi { ra: a, rs, sh, rc } => x(rs, a, sh, 824, rc),
        // [PPC-Book1 p:9 s:1.7.10 XS-Form] sh(16:20) XO(21:29) sh(30) Rc(31).
        I::Sradi { ra: a, rs, sh, rc } => {
            p(31)
                | rt(rs)
                | ra(a)
                | rb(sh)
                | (413 << 2)
                | ((((sh as u32) >> 5) & 1) << 1)
                | rc as u32
        }
        I::Cntlzw { ra: a, rs, rc } => x(rs, a, 0, 26, rc),
        I::Cntlzd { ra: a, rs, rc } => x(rs, a, 0, 58, rc),
        I::Extsh { ra: a, rs, rc } => x(rs, a, 0, 922, rc),
        I::Extsb { ra: a, rs, rc } => x(rs, a, 0, 954, rc),
        I::Extsw { ra: a, rs, rc } => x(rs, a, 0, 986, rc),
        I::Popcntb { ra: a, rs } => x(rs, a, 0, 122, false),
        I::Tw { to, ra: a, rb: b } => x(to, a, b, 4, false),
        I::Td { to, ra: a, rb: b } => x(to, a, b, 68, false),
        I::Mcrxr { bf } => x(bf << 2, 0, 0, 512, false),

        // X-form indexed loads and stores.
        I::Lwzx {
            rt: t,
            ra: a,
            rb: b,
        } => x(t, a, b, 23, false),
        I::Lbzx {
            rt: t,
            ra: a,
            rb: b,
        } => x(t, a, b, 87, false),
        I::Ldx {
            rt: t,
            ra: a,
            rb: b,
        } => x(t, a, b, 21, false),
        I::Lhzx {
            rt: t,
            ra: a,
            rb: b,
        } => x(t, a, b, 279, false),
        I::Lwzux {
            rt: t,
            ra: a,
            rb: b,
        } => x(t, a, b, 55, false),
        I::Lbzux {
            rt: t,
            ra: a,
            rb: b,
        } => x(t, a, b, 119, false),
        I::Lhzux {
            rt: t,
            ra: a,
            rb: b,
        } => x(t, a, b, 311, false),
        I::Ldux {
            rt: t,
            ra: a,
            rb: b,
        } => x(t, a, b, 53, false),
        I::Lhax {
            rt: t,
            ra: a,
            rb: b,
        } => x(t, a, b, 343, false),
        I::Lhaux {
            rt: t,
            ra: a,
            rb: b,
        } => x(t, a, b, 375, false),
        I::Lwax {
            rt: t,
            ra: a,
            rb: b,
        } => x(t, a, b, 341, false),
        I::Lwaux {
            rt: t,
            ra: a,
            rb: b,
        } => x(t, a, b, 373, false),
        I::Sthx { rs, ra: a, rb: b } => x(rs, a, b, 407, false),
        I::Sthux { rs, ra: a, rb: b } => x(rs, a, b, 439, false),
        I::Stwux { rs, ra: a, rb: b } => x(rs, a, b, 183, false),
        I::Stbux { rs, ra: a, rb: b } => x(rs, a, b, 247, false),
        I::Lswi { rt: t, ra: a, nb } => x(t, a, nb, 597, false),
        I::Stswi { rs, ra: a, nb } => x(rs, a, nb, 725, false),
        I::Lswx {
            rt: t,
            ra: a,
            rb: b,
        } => x(t, a, b, 533, false),
        I::Stswx { rs, ra: a, rb: b } => x(rs, a, b, 661, false),
        I::Ldarx {
            rt: t,
            ra: a,
            rb: b,
        } => x(t, a, b, 84, false),
        I::Stdcx { rs, ra: a, rb: b } => x(rs, a, b, 214, true),
        I::Lwarx {
            rt: t,
            ra: a,
            rb: b,
        } => x(t, a, b, 20, false),
        I::Stwcx { rs, ra: a, rb: b } => x(rs, a, b, 150, true),
        I::Stwx { rs, ra: a, rb: b } => x(rs, a, b, 151, false),
        I::Stdx { rs, ra: a, rb: b } => x(rs, a, b, 149, false),
        I::Stdux { rs, ra: a, rb: b } => x(rs, a, b, 181, false),
        I::Stbx { rs, ra: a, rb: b } => x(rs, a, b, 215, false),
        I::Ldbrx {
            rt: t,
            ra: a,
            rb: b,
        } => x(t, a, b, 532, false),
        I::Lwbrx {
            rt: t,
            ra: a,
            rb: b,
        } => x(t, a, b, 534, false),
        I::Sdbrx { rs, ra: a, rb: b } => x(rs, a, b, 660, false),
        I::Stwbrx { rs, ra: a, rb: b } => x(rs, a, b, 662, false),
        I::Lhbrx {
            rt: t,
            ra: a,
            rb: b,
        } => x(t, a, b, 790, false),
        I::Sthbrx { rs, ra: a, rb: b } => x(rs, a, b, 918, false),
        I::Dcbz { ra: a, rb: b } => x(0, a, b, 1014, false),

        // X-form floating-point loads and stores.
        I::Stfiwx { frs, ra: a, rb: b } => x(frs, a, b, 983, false),
        I::Lfsx { frt, ra: a, rb: b } => x(frt, a, b, 535, false),
        I::Lfsux { frt, ra: a, rb: b } => x(frt, a, b, 567, false),
        I::Lfdx { frt, ra: a, rb: b } => x(frt, a, b, 599, false),
        I::Lfdux { frt, ra: a, rb: b } => x(frt, a, b, 631, false),
        I::Stfsx { frs, ra: a, rb: b } => x(frs, a, b, 663, false),
        I::Stfsux { frs, ra: a, rb: b } => x(frs, a, b, 695, false),
        I::Stfdx { frs, ra: a, rb: b } => x(frs, a, b, 727, false),
        I::Stfdux { frs, ra: a, rb: b } => x(frs, a, b, 759, false),

        // X-form vector loads and stores.
        I::Lvlx { vt, ra: a, rb: b } => x(vt, a, b, 519, false),
        I::Lvrx { vt, ra: a, rb: b } => x(vt, a, b, 583, false),
        I::Lvlxl { vt, ra: a, rb: b } => x(vt, a, b, 647, false),
        I::Lvrxl { vt, ra: a, rb: b } => x(vt, a, b, 711, false),
        I::Stvlx { vs, ra: a, rb: b } => x(vs, a, b, 775, false),
        I::Stvrx { vs, ra: a, rb: b } => x(vs, a, b, 839, false),
        I::Stvlxl { vs, ra: a, rb: b } => x(vs, a, b, 903, false),
        I::Stvrxl { vs, ra: a, rb: b } => x(vs, a, b, 967, false),
        I::Lvsl { vt, ra: a, rb: b } => x(vt, a, b, 6, false),
        I::Lvebx { vt, ra: a, rb: b } => x(vt, a, b, 7, false),
        I::Lvsr { vt, ra: a, rb: b } => x(vt, a, b, 38, false),
        I::Lvehx { vt, ra: a, rb: b } => x(vt, a, b, 39, false),
        I::Lvewx { vt, ra: a, rb: b } => x(vt, a, b, 71, false),
        I::Lvx { vt, ra: a, rb: b } => x(vt, a, b, 103, false),
        I::Stvebx { vs, ra: a, rb: b } => x(vs, a, b, 135, false),
        I::Stvehx { vs, ra: a, rb: b } => x(vs, a, b, 167, false),
        I::Stvewx { vs, ra: a, rb: b } => x(vs, a, b, 199, false),
        I::Lvxl { vt, ra: a, rb: b } => x(vt, a, b, 359, false),
        I::Stvx { vs, ra: a, rb: b } => x(vs, a, b, 231, false),
        I::Stvxl { vs, ra: a, rb: b } => x(vs, a, b, 487, false),

        // XFX-form condition-register and special-purpose-register moves.
        // [PPC-Book1 p:83 s:3.3.16] mfcr is XO 19 and mtcrf is XO 144, each with bit 11 clear.
        // [PPC-Book1 p:163 s:B.9] mfocrf and mtocrf are the newer forms of the same mnemonics; the old forms carry bit 11 = 0.
        // [CBE-Handbook p:738 s:A.2.3.1] the PPE implements both one-field forms.
        I::Mfcr { rt: t } => x(t, 0, 0, 19, false),
        I::Mfocrf { rt: t, crm } => {
            p(31) | rt(t) | (1 << 20) | ((crm as u32) << 12) | xo_10(19, false)
        }
        I::Mtcrf { rs, crm } => p(31) | rt(rs) | ((crm as u32) << 12) | xo_10(144, false),
        I::Mtocrf { rs, crm } => {
            p(31) | rt(rs) | (1 << 20) | ((crm as u32) << 12) | xo_10(144, false)
        }
        I::Mfxer { rt: t } => xfx_spr(t, 1, 339),
        I::Mflr { rt: t } => xfx_spr(t, 8, 339),
        I::Mfctr { rt: t } => xfx_spr(t, 9, 339),
        I::Mfvrsave { rt: t } => xfx_spr(t, 256, 339),
        // [PPC-Book2 p:30 s:4.2 Reading the Time Base] mftb is the canonical spelling of TBR 268 and 269.
        I::Mftb { rt: t } => xfx_spr(t, 268, 371),
        I::Mftbu { rt: t } => xfx_spr(t, 269, 371),
        I::Mtxer { rs } => xfx_spr(rs, 1, 467),
        I::Mtlr { rs } => xfx_spr(rs, 8, 467),
        I::Mtctr { rs } => xfx_spr(rs, 9, 467),
        I::Mtvrsave { rs } => xfx_spr(rs, 256, 467),

        // M-form and MD-form rotates.
        I::Rlwimi {
            ra: a,
            rs,
            sh,
            mb,
            me,
            rc,
        } => m(20, rs, a, sh, mb, me, rc),
        I::Rlwinm {
            ra: a,
            rs,
            sh,
            mb,
            me,
            rc,
        } => m(21, rs, a, sh, mb, me, rc),
        I::Rlwnm {
            ra: a,
            rs,
            rb: b,
            mb,
            me,
            rc,
        } => m(23, rs, a, b, mb, me, rc),
        I::Rldicl {
            ra: a,
            rs,
            sh,
            mb,
            rc,
        } => md(rs, a, sh, mb, 0, rc),
        I::Rldicr {
            ra: a,
            rs,
            sh,
            me,
            rc,
        } => md(rs, a, sh, me, 1, rc),
        I::Rldic {
            ra: a,
            rs,
            sh,
            mb,
            rc,
        } => md(rs, a, sh, mb, 2, rc),
        I::Rldimi {
            ra: a,
            rs,
            sh,
            mb,
            rc,
        } => md(rs, a, sh, mb, 3, rc),
        I::Rldcl {
            ra: a,
            rs,
            rb: b,
            mb,
            rc,
        } => mds(rs, a, b, mb, 8, rc),
        I::Rldcr {
            ra: a,
            rs,
            rb: b,
            me,
            rc,
        } => mds(rs, a, b, me, 9, rc),

        // Vector forms under primary 4.
        // [AltiVec-PEM p:A-21 s:A.5 Table A-6 VX-Form] OPCD vD vA vB XO(21:31).
        I::Vx { op, rc, vt, va, vb } => {
            p(4) | rt(vt) | ra(va) | rb(vb) | (op as u32) | ((rc as u32) << 10)
        }
        I::Vxor { vt, va, vb } => p(4) | rt(vt) | ra(va) | rb(vb) | VxOp::Vxor as u32,
        // [AltiVec-PEM p:A-21 s:A.5 Table A-5 VA-Form] OPCD vD vA vB vC(21:25) XO(26:31).
        I::Va { op, vt, va, vb, vc } => p(4) | rt(vt) | ra(va) | rb(vb) | slot_6(vc) | op as u32,
        I::Vsldoi { vt, va, vb, shb } => {
            p(4) | rt(vt) | ra(va) | rb(vb) | slot_6(shb & 0xF) | VaOp::Vsldoi as u32
        }

        // Floating-point loads and stores.
        I::Lfs { frt, ra: a, imm } => d(48, frt, a, imm as u16),
        I::Lfsu { frt, ra: a, imm } => d(49, frt, a, imm as u16),
        I::Lfd { frt, ra: a, imm } => d(50, frt, a, imm as u16),
        I::Lfdu { frt, ra: a, imm } => d(51, frt, a, imm as u16),
        I::Stfs { frs, ra: a, imm } => d(52, frs, a, imm as u16),
        I::Stfsu { frs, ra: a, imm } => d(53, frs, a, imm as u16),
        I::Stfd { frs, ra: a, imm } => d(54, frs, a, imm as u16),
        I::Stfdu { frs, ra: a, imm } => d(55, frs, a, imm as u16),

        // Floating-point arithmetic. An X-form op under primary 63
        // carries its FRC slot inside the 10-bit XO, so the op alone
        // encodes it.
        I::Fp63 {
            op,
            frt,
            fra,
            frb,
            frc,
            rc,
        } => {
            let c = if op.is_a_form() { frc } else { 0 };
            a_form(63, frt, fra, frb, c, op as u32, rc)
        }
        I::Fp59 {
            op,
            frt,
            fra,
            frb,
            frc,
            rc,
        } => a_form(59, frt, fra, frb, frc, op as u32, rc),

        // [PPC-Book1 p:8 s:1.7.3 SC-Form] LEV(20:26) 1(30).
        I::Sc { lev } => p(17) | (((lev as u32) & 0x7F) << 5) | 2,

        // Quickened single instructions: the base form their extended mnemonic names.
        // [PPC-Book1 p:162 s:B.9 Load Immediate] li Rx,value == addi Rx,0,value.
        I::Li { rt: t, imm } => d(14, t, 0, imm as u16),
        // [PPC-Book1 p:163 s:B.9 Move Register] mr Rx,Ry == or Rx,Ry,Ry.
        I::Mr { ra: a, rs } => x(rs, a, rs, 444, false),
        // [PPC-Book1 p:160 s:B.7.2 Table 10] slwi n == rlwinm n,0,31-n; srwi n == rlwinm 32-n,n,31; clrlwi n == rlwinm 0,n,31.
        I::Slwi { ra: a, rs, n } => m(21, rs, a, n, 0, 31 - n, false),
        I::Srwi { ra: a, rs, n } => m(21, rs, a, 32 - n, n, 31, false),
        I::Clrlwi { ra: a, rs, n } => m(21, rs, a, 0, n, 31, false),
        // [PPC-Book1 p:159 s:B.7.1 Table 9] sldi n == rldicr n,63-n.
        // [PPC-Book1 p:160 s:B.7.1 Table 9] clrldi n == rldicl 0,n; srdi n == rldicl 64-n,n.
        I::Clrldi { ra: a, rs, n } => md(rs, a, 0, n, 0, false),
        I::Sldi { ra: a, rs, n } => md(rs, a, n, 63 - n, 1, false),
        I::Srdi { ra: a, rs, n } => md(rs, a, 64 - n, n, 0, false),
        // [PPC-Book1 p:162 s:B.9 No-op] the preferred no-op form is ori 0,0,0.
        I::Nop => d(24, 0, 0, 0),
        I::CmpwZero { bf, ra: a } => d(11, bf << 2, a, 0),

        I::LwzCmpwi { .. }
        | I::LiStw { .. }
        | I::MflrStw { .. }
        | I::LwzMtlr { .. }
        | I::MflrStd { .. }
        | I::LdMtlr { .. }
        | I::StdStd { .. }
        | I::CmpwiBc { .. }
        | I::CmpwBc { .. }
        | I::Consumed => {
            return Err(EncodeError::NoStandaloneEncoding {
                kind: PpuInstructionKind::from(*insn),
            })
        }
    })
}

/// Bits of a word carrying `insn` that the decoder does not read.
///
/// Two words that differ only there decode to the same `insn`. A
/// field the form reserves but the decoder stores, such as the FRA
/// slot of `fsqrt` or the vA slot of a unary VX op, is read and so
/// is not named here.
// [PPC-Book1 p:3 s:1.5.2 Reserved Fields and Reserved Values] a reserved field is ignored on read.
pub fn reserved_bits(insn: &PpuInstruction) -> u32 {
    use PpuInstruction as I;
    const RC: u32 = 0x0000_0001;
    const OE: u32 = 0x0000_0400;
    const RB: u32 = 0x0000_F800;
    const RA: u32 = 0x001F_0000;
    const RT: u32 = 0x03E0_0000;
    // PPC bit 9 of a compare, between BF and L.
    const CMP_BIT_9: u32 = 0x0040_0000;
    match *insn {
        I::Cmpwi { .. } | I::Cmpdi { .. } | I::Cmplwi { .. } | I::Cmpldi { .. } => CMP_BIT_9,
        I::Cmpw { .. } | I::Cmpd { .. } | I::Cmplw { .. } | I::Cmpld { .. } => CMP_BIT_9 | RC,
        // [PPC-Book1 p:8 s:1.7.3 SC-Form] every field but LEV and the marker bit is reserved.
        I::Sc { .. } => 0x03FF_F01C,
        // [PPC-Book1 p:30 s:2.4.4] mcrf reads BF and BFA only.
        I::Mcrf { .. } => 0x0063_F801,
        // [PPC-Book1 p:25 s:2.4.1] the BH hint and its reserved neighbours are not modelled.
        I::Bclr { .. } | I::Bcctr { .. } => RB,
        I::Crnor { .. }
        | I::Crandc { .. }
        | I::Crxor { .. }
        | I::Crnand { .. }
        | I::Crand { .. }
        | I::Creqv { .. }
        | I::Crorc { .. }
        | I::Cror { .. } => RC,
        I::Neg { .. } | I::Addze { .. } | I::Subfze { .. } | I::Subfme { .. } | I::Addme { .. } => {
            RB
        }
        I::Mulhwu { .. } | I::Mulhdu { .. } | I::Mulhd { .. } | I::Mulhw { .. } => OE,
        I::Cntlzw { .. }
        | I::Cntlzd { .. }
        | I::Extsh { .. }
        | I::Extsb { .. }
        | I::Extsw { .. } => RB,
        I::Popcntb { .. } => RB | RC,
        I::Tw { .. } | I::Td { .. } => RC,
        // [PPC-Book1 p:135 s:6.1] mcrxr reads BF only.
        I::Mcrxr { .. } => 0x0060_0000 | RA | RB | RC,
        I::Dcbz { .. } => RT | RC,
        I::Mfcr { .. } => 0x000F_F800 | RC,
        I::Mfocrf { .. } | I::Mtcrf { .. } | I::Mtocrf { .. } => 0x0000_0800 | RC,
        I::Lwzx { .. }
        | I::Lbzx { .. }
        | I::Ldx { .. }
        | I::Lhzx { .. }
        | I::Lwzux { .. }
        | I::Lbzux { .. }
        | I::Lhzux { .. }
        | I::Ldux { .. }
        | I::Lhax { .. }
        | I::Lhaux { .. }
        | I::Lwax { .. }
        | I::Lwaux { .. }
        | I::Sthx { .. }
        | I::Sthux { .. }
        | I::Stwux { .. }
        | I::Stbux { .. }
        | I::Lswi { .. }
        | I::Stswi { .. }
        | I::Lswx { .. }
        | I::Stswx { .. }
        | I::Ldarx { .. }
        | I::Lwarx { .. }
        | I::Stwx { .. }
        | I::Stdx { .. }
        | I::Stdux { .. }
        | I::Stbx { .. }
        | I::Ldbrx { .. }
        | I::Lwbrx { .. }
        | I::Sdbrx { .. }
        | I::Stwbrx { .. }
        | I::Lhbrx { .. }
        | I::Sthbrx { .. }
        | I::Stfiwx { .. }
        | I::Lfsx { .. }
        | I::Lfsux { .. }
        | I::Lfdx { .. }
        | I::Lfdux { .. }
        | I::Stfsx { .. }
        | I::Stfsux { .. }
        | I::Stfdx { .. }
        | I::Stfdux { .. }
        | I::Lvlx { .. }
        | I::Lvrx { .. }
        | I::Lvlxl { .. }
        | I::Lvrxl { .. }
        | I::Stvlx { .. }
        | I::Stvrx { .. }
        | I::Stvlxl { .. }
        | I::Stvrxl { .. }
        | I::Lvsl { .. }
        | I::Lvebx { .. }
        | I::Lvsr { .. }
        | I::Lvehx { .. }
        | I::Lvewx { .. }
        | I::Lvx { .. }
        | I::Stvebx { .. }
        | I::Stvehx { .. }
        | I::Stvewx { .. }
        | I::Lvxl { .. }
        | I::Stvx { .. }
        | I::Stvxl { .. }
        | I::Mfxer { .. }
        | I::Mflr { .. }
        | I::Mfctr { .. }
        | I::Mfvrsave { .. }
        | I::Mftb { .. }
        | I::Mftbu { .. }
        | I::Mtxer { .. }
        | I::Mtlr { .. }
        | I::Mtctr { .. }
        | I::Mtvrsave { .. } => RC,
        // [AltiVec-PEM p:6-136 s:6.2] vsldoi's SHB is four bits; bit 21 is reserved.
        I::Vsldoi { .. } => 0x0000_0400,
        I::Lwz { .. }
        | I::Lbz { .. }
        | I::Lhz { .. }
        | I::Lha { .. }
        | I::Lhau { .. }
        | I::Lmw { .. }
        | I::Lwzu { .. }
        | I::Lbzu { .. }
        | I::Lhzu { .. }
        | I::Ldu { .. }
        | I::Ld { .. }
        | I::Lwa { .. }
        | I::Stw { .. }
        | I::Stwu { .. }
        | I::Stdu { .. }
        | I::Stb { .. }
        | I::Stbu { .. }
        | I::Stmw { .. }
        | I::Sth { .. }
        | I::Sthu { .. }
        | I::Std { .. }
        | I::Addi { .. }
        | I::Addis { .. }
        | I::Subfic { .. }
        | I::Mulli { .. }
        | I::Addic { .. }
        | I::AddicDot { .. }
        | I::Add { .. }
        | I::Or { .. }
        | I::Subf { .. }
        | I::Subfc { .. }
        | I::Subfe { .. }
        | I::Mullw { .. }
        | I::Adde { .. }
        | I::Mulld { .. }
        | I::Stdcx { .. }
        | I::Stwcx { .. }
        | I::Xori { .. }
        | I::Xoris { .. }
        | I::Divw { .. }
        | I::Divwu { .. }
        | I::Divd { .. }
        | I::Divdu { .. }
        | I::And { .. }
        | I::Andc { .. }
        | I::Nor { .. }
        | I::Xor { .. }
        | I::Eqv { .. }
        | I::Nand { .. }
        | I::AndiDot { .. }
        | I::AndisDot { .. }
        | I::Slw { .. }
        | I::Srw { .. }
        | I::Srawi { .. }
        | I::Sraw { .. }
        | I::Srad { .. }
        | I::Sradi { .. }
        | I::Sld { .. }
        | I::Srd { .. }
        | I::Orc { .. }
        | I::Ori { .. }
        | I::Oris { .. }
        | I::B { .. }
        | I::Bc { .. }
        | I::Rlwinm { .. }
        | I::Rlwimi { .. }
        | I::Rlwnm { .. }
        | I::Rldicl { .. }
        | I::Rldicr { .. }
        | I::Rldic { .. }
        | I::Rldimi { .. }
        | I::Rldcl { .. }
        | I::Rldcr { .. }
        | I::Vx { .. }
        | I::Va { .. }
        | I::Vxor { .. }
        | I::Lfs { .. }
        | I::Lfsu { .. }
        | I::Lfd { .. }
        | I::Lfdu { .. }
        | I::Stfs { .. }
        | I::Stfd { .. }
        | I::Stfsu { .. }
        | I::Stfdu { .. }
        | I::Fp63 { .. }
        | I::Fp59 { .. }
        | I::Li { .. }
        | I::Mr { .. }
        | I::Slwi { .. }
        | I::Srwi { .. }
        | I::Clrlwi { .. }
        | I::Nop
        | I::CmpwZero { .. }
        | I::Clrldi { .. }
        | I::Sldi { .. }
        | I::Srdi { .. }
        | I::LwzCmpwi { .. }
        | I::LiStw { .. }
        | I::MflrStw { .. }
        | I::LwzMtlr { .. }
        | I::MflrStd { .. }
        | I::LdMtlr { .. }
        | I::StdStd { .. }
        | I::CmpwiBc { .. }
        | I::CmpwBc { .. }
        | I::Consumed => 0,
    }
}

/// Names the second spelling `raw` uses when it is not the canonical word.
///
/// The deterministic model decodes `isync` and the storage-control
/// hints as `nop`. Returns `None` for a word whose only difference
/// from its canonical encoding is in [`reserved_bits`].
pub fn alias(raw: u32) -> Option<Alias> {
    let primary = (raw >> 26) as u8;
    let xo = ((raw >> 1) & 0x3FF) as u16;
    match primary {
        19 if u32::from(xo) == PPC_ISYNC_XO => Some(Alias::NopHint { primary, xo }),
        31 if PPC_STORAGE_HINT_XOS.contains(&u32::from(xo)) => Some(Alias::NopHint { primary, xo }),
        31 if xo == 339 => {
            let spr = (((raw >> 11) & 0x1F) << 5 | ((raw >> 16) & 0x1F)) as u16;
            (spr == 268 || spr == 269).then_some(Alias::TimeBaseThroughMfspr { tbr: spr })
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "tests/encode_tests.rs"]
mod tests;
