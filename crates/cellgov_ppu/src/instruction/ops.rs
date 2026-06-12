//! Op enums for the family-dispatch decode variants
//! (`Vx` / `Va` / `Fp59` / `Fp63`).
//!
//! Each enum carries the documented extended opcodes as explicit
//! discriminants; the variant name doubles as the mnemonic via
//! `strum::IntoStaticStr` with lowercase serialization, so there is
//! no separate string table to scramble. Any edit must keep the
//! encoding-law tests in this module green; they re-derive the
//! AltiVec opcode-map geometry (width stride, signedness offset,
//! saturate offset, VXR pairing) in a second, independent shape.
//!
//! VX / VA discriminants transcribed from AltiVec-PEM Appendix A.5;
//! opcode-59 / opcode-63 discriminants from the PPC v2.02 Book I
//! floating-point chapters.
// [AltiVec-PEM p:A-21 s:A.5] primary-4 opcode tables (VA 6-bit XO,
// VX 11-bit XO, VXR 10-bit XO + Rc).
// [PPC-Book1 p:84 s:4.6] floating-point instruction set, primary
// opcodes 59 and 63.

/// VX-form (and VXR-form compare) extended opcodes under primary 4.
///
/// VXR compares appear once, as their Rc=0 base XO; a word carrying
/// `1024 | base` decodes as the base op with `rc: true` via
/// [`VxOp::decode`]. Non-compare ops whose discriminant has bit 10
/// set (`vand`, `vor`, ...) are ordinary VX ops and never carry Rc.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, strum::IntoStaticStr, strum::EnumIter, strum::FromRepr,
)]
#[strum(serialize_all = "lowercase")]
#[repr(u16)]
#[allow(missing_docs)]
pub enum VxOp {
    Vaddubm = 0,
    Vmaxub = 2,
    Vrlb = 4,
    Vcmpequb = 6,
    Vmuloub = 8,
    Vaddfp = 10,
    Vmrghb = 12,
    Vpkuhum = 14,
    Vadduhm = 64,
    Vmaxuh = 66,
    Vrlh = 68,
    Vcmpequh = 70,
    Vmulouh = 72,
    Vsubfp = 74,
    Vmrghh = 76,
    Vpkuwum = 78,
    Vadduwm = 128,
    Vmaxuw = 130,
    Vrlw = 132,
    Vcmpequw = 134,
    Vmrghw = 140,
    Vpkuhus = 142,
    Vcmpeqfp = 198,
    Vpkuwus = 206,
    Vmaxsb = 258,
    Vslb = 260,
    Vmulosb = 264,
    Vrefp = 266,
    Vmrglb = 268,
    Vpkshus = 270,
    Vmaxsh = 322,
    Vslh = 324,
    Vmulosh = 328,
    Vrsqrtefp = 330,
    Vmrglh = 332,
    Vpkswus = 334,
    Vaddcuw = 384,
    Vmaxsw = 386,
    Vslw = 388,
    Vexptefp = 394,
    Vmrglw = 396,
    Vpkshss = 398,
    Vsl = 452,
    Vcmpgefp = 454,
    Vlogefp = 458,
    Vpkswss = 462,
    Vaddubs = 512,
    Vminub = 514,
    Vsrb = 516,
    Vcmpgtub = 518,
    Vmuleub = 520,
    Vrfin = 522,
    Vspltb = 524,
    Vupkhsb = 526,
    Vadduhs = 576,
    Vminuh = 578,
    Vsrh = 580,
    Vcmpgtuh = 582,
    Vmuleuh = 584,
    Vrfiz = 586,
    Vsplth = 588,
    Vupkhsh = 590,
    Vadduws = 640,
    Vminuw = 642,
    Vsrw = 644,
    Vcmpgtuw = 646,
    Vrfip = 650,
    Vspltw = 652,
    Vupklsb = 654,
    Vsr = 708,
    Vcmpgtfp = 710,
    Vrfim = 714,
    Vupklsh = 718,
    Vaddsbs = 768,
    Vminsb = 770,
    Vsrab = 772,
    Vcmpgtsb = 774,
    Vmulesb = 776,
    Vcfux = 778,
    Vspltisb = 780,
    Vpkpx = 782,
    Vaddshs = 832,
    Vminsh = 834,
    Vsrah = 836,
    Vcmpgtsh = 838,
    Vmulesh = 840,
    Vcfsx = 842,
    Vspltish = 844,
    Vupkhpx = 846,
    Vaddsws = 896,
    Vminsw = 898,
    Vsraw = 900,
    Vcmpgtsw = 902,
    Vctuxs = 906,
    Vspltisw = 908,
    Vcmpbfp = 966,
    Vctsxs = 970,
    Vupklpx = 974,
    Vsububm = 1024,
    Vavgub = 1026,
    Vand = 1028,
    Vmaxfp = 1034,
    Vslo = 1036,
    Vsubuhm = 1088,
    Vavguh = 1090,
    Vandc = 1092,
    Vminfp = 1098,
    Vsro = 1100,
    Vsubuwm = 1152,
    Vavguw = 1154,
    Vor = 1156,
    Vxor = 1220,
    Vavgsb = 1282,
    Vnor = 1284,
    Vavgsh = 1346,
    Vsubcuw = 1408,
    Vavgsw = 1410,
    Vsububs = 1536,
    Vsum4ubs = 1544,
    Vsubuhs = 1600,
    Vsum4shs = 1608,
    Vsubuws = 1664,
    Vsum2sws = 1672,
    Vsubsbs = 1792,
    Vsum4sbs = 1800,
    Vsubshs = 1856,
    Vsubsws = 1920,
    Vsumsws = 1928,
}

/// Operand layout of a VX-form op, keyed for the formatter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VxShape {
    /// `vD, vA, vB` -- the standard three-register form.
    VdVaVb,
    /// `vD, vB` -- unary ops with the vA slot reserved.
    VdVb,
    /// `vD, vB, UIMM` -- splat / convert ops carrying an unsigned
    /// immediate in the vA slot.
    VdVbUimm,
    /// `vD, SIMM` -- splat-immediate ops carrying a sign-extended
    /// 5-bit immediate in the vA slot.
    VdSimm,
}

impl VxOp {
    /// Resolve an 11-bit primary-4 extended opcode to `(op, rc)`.
    ///
    /// Direct discriminant hits win (so `vand` at 1028 is never
    /// misread as an Rc form); otherwise bit 10 is stripped and the
    /// remainder accepted only if it names a VXR compare base.
    pub fn decode(xo: u16) -> Option<(VxOp, bool)> {
        if let Some(op) = VxOp::from_repr(xo) {
            return Some((op, false));
        }
        if xo & 0x400 != 0 {
            if let Some(op) = VxOp::from_repr(xo & 0x3FF) {
                if op.is_vxr_compare() {
                    return Some((op, true));
                }
            }
        }
        None
    }

    /// Whether this op is a VXR-form compare (carries an Rc bit).
    pub fn is_vxr_compare(self) -> bool {
        matches!(
            self,
            Self::Vcmpequb
                | Self::Vcmpequh
                | Self::Vcmpequw
                | Self::Vcmpeqfp
                | Self::Vcmpgefp
                | Self::Vcmpgtub
                | Self::Vcmpgtuh
                | Self::Vcmpgtuw
                | Self::Vcmpgtfp
                | Self::Vcmpgtsb
                | Self::Vcmpgtsh
                | Self::Vcmpgtsw
                | Self::Vcmpbfp
        )
    }

    /// Operand layout for rendering. Exhaustive: a newly added op
    /// fails compilation here until its shape is decided.
    pub fn shape(self) -> VxShape {
        match self {
            Self::Vrefp
            | Self::Vrsqrtefp
            | Self::Vexptefp
            | Self::Vlogefp
            | Self::Vrfin
            | Self::Vrfiz
            | Self::Vrfip
            | Self::Vrfim
            | Self::Vupkhsb
            | Self::Vupkhsh
            | Self::Vupklsb
            | Self::Vupklsh
            | Self::Vupkhpx
            | Self::Vupklpx => VxShape::VdVb,
            Self::Vspltb
            | Self::Vsplth
            | Self::Vspltw
            | Self::Vcfux
            | Self::Vcfsx
            | Self::Vctuxs
            | Self::Vctsxs => VxShape::VdVbUimm,
            Self::Vspltisb | Self::Vspltish | Self::Vspltisw => VxShape::VdSimm,
            Self::Vaddubm
            | Self::Vmaxub
            | Self::Vrlb
            | Self::Vcmpequb
            | Self::Vmuloub
            | Self::Vaddfp
            | Self::Vmrghb
            | Self::Vpkuhum
            | Self::Vadduhm
            | Self::Vmaxuh
            | Self::Vrlh
            | Self::Vcmpequh
            | Self::Vmulouh
            | Self::Vsubfp
            | Self::Vmrghh
            | Self::Vpkuwum
            | Self::Vadduwm
            | Self::Vmaxuw
            | Self::Vrlw
            | Self::Vcmpequw
            | Self::Vmrghw
            | Self::Vpkuhus
            | Self::Vcmpeqfp
            | Self::Vpkuwus
            | Self::Vmaxsb
            | Self::Vslb
            | Self::Vmulosb
            | Self::Vmrglb
            | Self::Vpkshus
            | Self::Vmaxsh
            | Self::Vslh
            | Self::Vmulosh
            | Self::Vmrglh
            | Self::Vpkswus
            | Self::Vaddcuw
            | Self::Vmaxsw
            | Self::Vslw
            | Self::Vmrglw
            | Self::Vpkshss
            | Self::Vsl
            | Self::Vcmpgefp
            | Self::Vpkswss
            | Self::Vaddubs
            | Self::Vminub
            | Self::Vsrb
            | Self::Vcmpgtub
            | Self::Vmuleub
            | Self::Vadduhs
            | Self::Vminuh
            | Self::Vsrh
            | Self::Vcmpgtuh
            | Self::Vmuleuh
            | Self::Vadduws
            | Self::Vminuw
            | Self::Vsrw
            | Self::Vcmpgtuw
            | Self::Vsr
            | Self::Vcmpgtfp
            | Self::Vaddsbs
            | Self::Vminsb
            | Self::Vsrab
            | Self::Vcmpgtsb
            | Self::Vmulesb
            | Self::Vpkpx
            | Self::Vaddshs
            | Self::Vminsh
            | Self::Vsrah
            | Self::Vcmpgtsh
            | Self::Vmulesh
            | Self::Vaddsws
            | Self::Vminsw
            | Self::Vsraw
            | Self::Vcmpgtsw
            | Self::Vcmpbfp
            | Self::Vsububm
            | Self::Vavgub
            | Self::Vand
            | Self::Vmaxfp
            | Self::Vslo
            | Self::Vsubuhm
            | Self::Vavguh
            | Self::Vandc
            | Self::Vminfp
            | Self::Vsro
            | Self::Vsubuwm
            | Self::Vavguw
            | Self::Vor
            | Self::Vxor
            | Self::Vavgsb
            | Self::Vnor
            | Self::Vavgsh
            | Self::Vsubcuw
            | Self::Vavgsw
            | Self::Vsububs
            | Self::Vsum4ubs
            | Self::Vsubuhs
            | Self::Vsum4shs
            | Self::Vsubuws
            | Self::Vsum2sws
            | Self::Vsubsbs
            | Self::Vsum4sbs
            | Self::Vsubshs
            | Self::Vsubsws
            | Self::Vsumsws => VxShape::VdVaVb,
        }
    }
}

/// VA-form 6-bit extended opcodes under primary 4.
///
/// `vsldoi` decodes to the typed
/// [`PpuInstruction::Vsldoi`](super::PpuInstruction::Vsldoi)
/// before this enum is consulted; the variant exists so formatter
/// and executor matches stay total.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, strum::IntoStaticStr, strum::EnumIter, strum::FromRepr,
)]
#[strum(serialize_all = "lowercase")]
#[repr(u8)]
#[allow(missing_docs)]
pub enum VaOp {
    Vmhaddshs = 32,
    Vmhraddshs = 33,
    Vmladduhm = 34,
    Vmsumubm = 36,
    Vmsummbm = 37,
    Vmsumuhm = 38,
    Vmsumuhs = 39,
    Vmsumshm = 40,
    Vmsumshs = 41,
    Vsel = 42,
    Vperm = 43,
    Vsldoi = 44,
    Vmaddfp = 46,
    Vnmsubfp = 47,
}

/// Operand layout of a VA-form op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaShape {
    /// `vD, vA, vB, vC`.
    VdVaVbVc,
    /// `vD, vA, vC, vB` -- the FP multiply-add syntax puts the
    /// multiplier (vC) before the addend (vB).
    VdVaVcVb,
    /// `vD, vA, vB, SHB` (`vsldoi`).
    VdVaVbShb,
}

impl VaOp {
    /// Operand layout for rendering.
    pub fn shape(self) -> VaShape {
        match self {
            Self::Vmaddfp | Self::Vnmsubfp => VaShape::VdVaVcVb,
            Self::Vsldoi => VaShape::VdVaVbShb,
            Self::Vmhaddshs
            | Self::Vmhraddshs
            | Self::Vmladduhm
            | Self::Vmsumubm
            | Self::Vmsummbm
            | Self::Vmsumuhm
            | Self::Vmsumuhs
            | Self::Vmsumshm
            | Self::Vmsumshs
            | Self::Vsel
            | Self::Vperm => VaShape::VdVaVbVc,
        }
    }
}

/// Primary-63 (double-precision FP and FPSCR) extended opcodes.
///
/// A-form arithmetic ops store their 5-bit XO (the FRC field rides
/// above it in the 10-bit extraction); X-form ops store the full
/// 10-bit XO. The two ranges are disjoint, so one repr space works;
/// [`Fp63Op::from_xo`] resolves the split.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, strum::IntoStaticStr, strum::EnumIter, strum::FromRepr,
)]
#[strum(serialize_all = "lowercase")]
#[repr(u16)]
#[allow(missing_docs)]
pub enum Fp63Op {
    Fcmpu = 0,
    Frsp = 12,
    Fctiw = 14,
    Fctiwz = 15,
    Fdiv = 18,
    Fsub = 20,
    Fadd = 21,
    Fsqrt = 22,
    Fsel = 23,
    Fmul = 25,
    Frsqrte = 26,
    Fmsub = 28,
    Fmadd = 29,
    Fnmsub = 30,
    Fnmadd = 31,
    Fcmpo = 32,
    Mtfsb1 = 38,
    Fneg = 40,
    Mcrfs = 64,
    Mtfsb0 = 70,
    Fmr = 72,
    Mtfsfi = 134,
    Fnabs = 136,
    Fabs = 264,
    Mffs = 583,
    Mtfsf = 711,
    Fctid = 814,
    Fctidz = 815,
    Fcfid = 846,
}

/// Operand layout of a primary-63 op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fp63Shape {
    /// `frD, frA, frB`.
    FrtFraFrb,
    /// `frD, frB`.
    FrtFrb,
    /// `frD, frA, frC` (`fmul`).
    FrtFraFrc,
    /// `frD, frA, frC, frB` (fused multiply-add family, `fsel`).
    FrtFraFrcFrb,
    /// `crfD, frA, frB` (compares).
    CrfFraFrb,
    /// `frD` (`mffs`).
    Frt,
    /// `crfD, crfS` (`mcrfs`).
    CrfCrf,
    /// `crfD, U` (`mtfsfi`).
    CrfImm,
    /// `crbD` (`mtfsb0` / `mtfsb1`).
    Crb,
    /// `FM, frB` (`mtfsf`).
    FmFrb,
}

impl Fp63Op {
    /// Whether this op is A-form (5-bit XO; FRC is an operand).
    fn is_a_form(self) -> bool {
        matches!(
            self,
            Self::Fdiv
                | Self::Fsub
                | Self::Fadd
                | Self::Fsqrt
                | Self::Fsel
                | Self::Fmul
                | Self::Frsqrte
                | Self::Fmsub
                | Self::Fmadd
                | Self::Fnmsub
                | Self::Fnmadd
        )
    }

    /// Resolve a 10-bit primary-63 extended opcode.
    ///
    /// A-form ops match on the low 5 bits (the upper 5 carry FRC);
    /// everything else matches the full 10-bit value. Safe because
    /// no X-form discriminant's low 5 bits collide with an A-form
    /// discriminant.
    pub fn from_xo(xo: u16) -> Option<Fp63Op> {
        if let Some(op) = Fp63Op::from_repr(xo & 0x1F) {
            if op.is_a_form() {
                return Some(op);
            }
        }
        Fp63Op::from_repr(xo)
    }

    /// Operand layout for rendering.
    pub fn shape(self) -> Fp63Shape {
        match self {
            Self::Fdiv | Self::Fsub | Self::Fadd => Fp63Shape::FrtFraFrb,
            Self::Fsqrt
            | Self::Frsqrte
            | Self::Frsp
            | Self::Fctiw
            | Self::Fctiwz
            | Self::Fctid
            | Self::Fctidz
            | Self::Fcfid
            | Self::Fmr
            | Self::Fneg
            | Self::Fabs
            | Self::Fnabs => Fp63Shape::FrtFrb,
            Self::Fmul => Fp63Shape::FrtFraFrc,
            Self::Fsel | Self::Fmadd | Self::Fmsub | Self::Fnmadd | Self::Fnmsub => {
                Fp63Shape::FrtFraFrcFrb
            }
            Self::Fcmpu | Self::Fcmpo => Fp63Shape::CrfFraFrb,
            Self::Mffs => Fp63Shape::Frt,
            Self::Mcrfs => Fp63Shape::CrfCrf,
            Self::Mtfsfi => Fp63Shape::CrfImm,
            Self::Mtfsb0 | Self::Mtfsb1 => Fp63Shape::Crb,
            Self::Mtfsf => Fp63Shape::FmFrb,
        }
    }
}

/// Primary-59 (single-precision FP) extended opcodes. All A-form;
/// the discriminant is the 5-bit XO.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, strum::IntoStaticStr, strum::EnumIter, strum::FromRepr,
)]
#[strum(serialize_all = "lowercase")]
#[repr(u16)]
#[allow(missing_docs)]
pub enum Fp59Op {
    Fdivs = 18,
    Fsubs = 20,
    Fadds = 21,
    Fsqrts = 22,
    Fres = 24,
    Fmuls = 25,
    Fmsubs = 28,
    Fmadds = 29,
    Fnmsubs = 30,
    Fnmadds = 31,
}

/// Operand layout of a primary-59 op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fp59Shape {
    /// `frD, frA, frB`.
    FrtFraFrb,
    /// `frD, frB`.
    FrtFrb,
    /// `frD, frA, frC` (`fmuls`).
    FrtFraFrc,
    /// `frD, frA, frC, frB`.
    FrtFraFrcFrb,
}

impl Fp59Op {
    /// Resolve a 10-bit primary-59 extended opcode (low 5 bits).
    pub fn from_xo(xo: u16) -> Option<Fp59Op> {
        Fp59Op::from_repr(xo & 0x1F)
    }

    /// Operand layout for rendering.
    pub fn shape(self) -> Fp59Shape {
        match self {
            Self::Fdivs | Self::Fsubs | Self::Fadds => Fp59Shape::FrtFraFrb,
            Self::Fsqrts | Self::Fres => Fp59Shape::FrtFrb,
            Self::Fmuls => Fp59Shape::FrtFraFrc,
            Self::Fmadds | Self::Fmsubs | Self::Fnmadds | Self::Fnmsubs => Fp59Shape::FrtFraFrcFrb,
        }
    }
}

#[cfg(test)]
#[path = "tests/ops_tests.rs"]
mod tests;
