//! The FPSCR move instructions, the summary bits every FPSCR write
//! recomputes, and the status writeback of a computational instruction.
//!
//! Each move instruction takes its fields from the primary-63 decode:
//! `frt` is instruction bits 6:10, `fra` bits 11:15 and `frb` bits 16:20.

use cellgov_float::{PpcControl, PpcEnables, PpcInvalid, PpcPrecision, PpcStatus, Rounding};
use cellgov_ps3_abi::hw::ppc_isa::{
    PPC_FPSCR_ENABLE_BITS, PPC_FPSCR_EXCEPTION_BITS, PPC_FPSCR_FEX, PPC_FPSCR_FI, PPC_FPSCR_FPCC,
    PPC_FPSCR_FPRF, PPC_FPSCR_FR, PPC_FPSCR_FX, PPC_FPSCR_OE, PPC_FPSCR_OX, PPC_FPSCR_RN,
    PPC_FPSCR_UE, PPC_FPSCR_UX, PPC_FPSCR_VE, PPC_FPSCR_VX, PPC_FPSCR_VXCVI, PPC_FPSCR_VXIDI,
    PPC_FPSCR_VXIMZ, PPC_FPSCR_VXISI, PPC_FPSCR_VXSNAN, PPC_FPSCR_VXSQRT, PPC_FPSCR_VXVC,
    PPC_FPSCR_VXZDZ, PPC_FPSCR_VX_BITS, PPC_FPSCR_XE, PPC_FPSCR_XX, PPC_FPSCR_ZE, PPC_FPSCR_ZX,
    PPE_FPSCR_UNIMPLEMENTED_BITS, PPE_MFFS_HIGH_WORD,
};

use crate::state::PpuState;

/// The class of a result, as FPRF records it: the class descriptor C
/// then FPCC's <, >, = and ?.
///
/// [PPC-Book1 p:89 s:4.2.2] Figure 29 gives the five FPRF bits of each result class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub(crate) enum Fprf {
    QuietNan = 0b10001,
    NegativeInfinity = 0b01001,
    NegativeNormal = 0b01000,
    NegativeDenormal = 0b11000,
    NegativeZero = 0b10010,
    PositiveZero = 0b00010,
    PositiveDenormal = 0b10100,
    PositiveNormal = 0b00100,
    PositiveInfinity = 0b00101,
}

impl Fprf {
    /// The class of the double-format result `bits` of a `precision`
    /// operation. A single-precision value below 2^-126 is denormalized
    /// in single format, although double format stores it normalized.
    ///
    /// [PPC-Book1 p:88 s:4.2.2] a single-precision operation with a denormalized result sets FPRF to a denormalized number.
    pub(crate) fn of(bits: u64, precision: PpcPrecision) -> Fprf {
        let negative = bits >> 63 == 1;
        let biased = (bits >> 52 & 0x7FF) as u32;
        let fraction = bits & ((1 << 52) - 1);
        // The lowest biased double exponent of a normalized result.
        let normal_from = match precision {
            PpcPrecision::Double => 1,
            PpcPrecision::Single => 1023 - 126,
        };
        match (biased, fraction, negative) {
            (0x7FF, 0, false) => Fprf::PositiveInfinity,
            (0x7FF, 0, true) => Fprf::NegativeInfinity,
            (0x7FF, _, _) => Fprf::QuietNan,
            (0, 0, false) => Fprf::PositiveZero,
            (0, 0, true) => Fprf::NegativeZero,
            (e, _, false) if e < normal_from => Fprf::PositiveDenormal,
            (e, _, true) if e < normal_from => Fprf::NegativeDenormal,
            (_, _, false) => Fprf::PositiveNormal,
            (_, _, true) => Fprf::NegativeNormal,
        }
    }
}

/// Writes one computational instruction's status into the FPSCR.
///
/// It writes:
///
/// - the sticky exception bits;
/// - FX, when one of them turns from 0 to 1;
/// - FR and FI, with FI ORed into XX;
/// - FPRF;
/// - the VX and FEX summaries.
///
/// [PPC-Book1 p:87 s:4.2.2] the exception bits are sticky; FX is set when an instruction turns an exception bit from 0 to 1; XX ORs in each new FI.
/// [PPC-Book1 p:88 s:4.2.2] FR and FI are not sticky; FPRF is set from the result and the target precision.
/// [PPC-Book1 p:97 s:4.4.1] an invalid operation sets one bit per cause.
pub(crate) fn write_status(state: &mut PpuState, status: &PpcStatus, fprf: Fprf) {
    write_status_fields(state, status, Some(fprf));
}

/// Writes what [`write_status`] writes, with FPRF unchanged.
///
/// The callers write the status of:
///
/// - a conversion to integer;
/// - an operation whose enabled exception leaves FRT unchanged.
///
/// [PPC-Book1 p:117 s:4.6.6] a convert to integer leaves FPRF undefined.
/// The ppu_fp_convert_modes console capture reads the FPRF bits the instruction started with.
/// [PPC-Book1 p:97 s:4.4.1] with VE = 1 an invalid operation leaves FPRF unchanged.
/// [PPC-Book1 p:98 s:4.4.2] with ZE = 1 a zero divide leaves FPRF unchanged.
pub(crate) fn write_status_keeping_fprf(state: &mut PpuState, status: &PpcStatus) {
    write_status_fields(state, status, None);
}

/// [`write_status`], with FPRF replaced when `fprf` names a class.
fn write_status_fields(state: &mut PpuState, status: &PpcStatus, fprf: Option<Fprf>) {
    let old = state.fpscr();
    let invalid = &status.invalid;
    let mut raised = 0;
    for (set, bit) in [
        (status.ox, PPC_FPSCR_OX),
        (status.ux, PPC_FPSCR_UX),
        (status.zx, PPC_FPSCR_ZX),
        (invalid.snan, PPC_FPSCR_VXSNAN),
        (invalid.inf_minus_inf, PPC_FPSCR_VXISI),
        (invalid.inf_div_inf, PPC_FPSCR_VXIDI),
        (invalid.zero_div_zero, PPC_FPSCR_VXZDZ),
        (invalid.inf_times_zero, PPC_FPSCR_VXIMZ),
        (invalid.compare, PPC_FPSCR_VXVC),
        (invalid.sqrt, PPC_FPSCR_VXSQRT),
        (invalid.convert, PPC_FPSCR_VXCVI),
    ] {
        if set {
            raised |= bit;
        }
    }
    let mut v = old & !(PPC_FPSCR_FR | PPC_FPSCR_FI);
    if let Some(fprf) = fprf {
        v = v & !PPC_FPSCR_FPRF | (fprf as u32) << 12;
    }
    if status.fr {
        v |= PPC_FPSCR_FR;
    }
    if status.fi {
        v |= PPC_FPSCR_FI;
        raised |= PPC_FPSCR_XX;
    }
    v |= raised;
    if raised & !old != 0 {
        v |= PPC_FPSCR_FX;
    }
    state.set_fpscr(with_summaries(v));
}

/// Writes a compare's status into the FPSCR.
///
/// It writes:
///
/// - FPCC, from `fpcc`;
/// - VXSNAN and VXVC, as `invalid.snan` and `invalid.compare` raise them;
/// - FX, when one of them turns from 0 to 1;
/// - the VX and FEX summaries.
///
/// C, FR and FI keep their values.
///
/// [PPC-Book1 p:119 s:4.6.7] fcmpu and fcmpo alter FPCC, FX, VXSNAN and VXVC.
/// [PPC-Book1 p:97 s:4.4.1] a compare with an invalid operation leaves FR, FI and C unchanged.
pub(crate) fn write_compare_status(state: &mut PpuState, fpcc: u8, invalid: &PpcInvalid) {
    let old = state.fpscr();
    let mut raised = 0;
    if invalid.snan {
        raised |= PPC_FPSCR_VXSNAN;
    }
    if invalid.compare {
        raised |= PPC_FPSCR_VXVC;
    }
    let mut v = (old & !PPC_FPSCR_FPCC) | raised | u32::from(fpcc & 0xF) << 12;
    if raised & !old != 0 {
        v |= PPC_FPSCR_FX;
    }
    state.set_fpscr(with_summaries(v));
}

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

/// The rounding mode FPSCR RN selects.
///
/// The ppu_fp_addmul_modes console capture rounds by this encoding in all four modes.
pub(crate) fn rounding(fpscr: u32) -> Rounding {
    match fpscr & PPC_FPSCR_RN {
        0 => Rounding::NearestEven,
        1 => Rounding::TowardZero,
        2 => Rounding::TowardPositive,
        _ => Rounding::TowardNegative,
    }
}

/// The rounding mode and the exception enables `fpscr` holds.
pub(crate) fn control(fpscr: u32) -> PpcControl {
    PpcControl {
        rounding: rounding(fpscr),
        enables: PpcEnables {
            invalid: fpscr & PPC_FPSCR_VE != 0,
            overflow: fpscr & PPC_FPSCR_OE != 0,
            underflow: fpscr & PPC_FPSCR_UE != 0,
            zero_divide: fpscr & PPC_FPSCR_ZE != 0,
            inexact: fpscr & PPC_FPSCR_XE != 0,
        },
    }
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
#[path = "tests/fpscr_enabled_console_tests.rs"]
mod fpscr_enabled_console_tests;

#[cfg(test)]
#[path = "tests/fpscr_console_tests.rs"]
mod console_tests;
