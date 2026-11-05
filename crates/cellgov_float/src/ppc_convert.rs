//! The PowerPC conversions, with every exception disabled:
//!
//! - a double to a 32-bit or 64-bit integer
//! - a 64-bit integer to a double
//! - a double to single precision

use crate::format::Binary64;
use crate::operand::{unpack, Operand};
use crate::ppc::{
    ppc_frsp_nan, ppc_round_disabled, PpcInvalid, PpcPrecision, PpcResult, PpcStatus,
};
use crate::round::{Exact, Policy, Rounding};

/// The width of the integer a convert to integer gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpcInteger {
    /// A 32-bit integer: fctiw and fctiwz.
    Word,
    /// A 64-bit integer: fctid and fctidz.
    Doubleword,
}

impl PpcInteger {
    /// The most positive integer, and the magnitude of the most negative.
    fn limits(self) -> (u128, u128) {
        match self {
            Self::Word => (0x7FFF_FFFF, 0x8000_0000),
            Self::Doubleword => (0x7FFF_FFFF_FFFF_FFFF, 0x8000_0000_0000_0000),
        }
    }

    /// The result of an invalid conversion: the most negative integer for
    /// a negative operand or a NaN, else the most positive.
    fn saturated(self, negative: bool) -> u64 {
        match (self, negative) {
            (Self::Word, false) => 0x7FFF_FFFF,
            (Self::Word, true) => 0xFFFF_FFFF_8000_0000,
            (Self::Doubleword, false) => 0x7FFF_FFFF_FFFF_FFFF,
            (Self::Doubleword, true) => 0x8000_0000_0000_0000,
        }
    }
}

/// The double-format `b` rounded in `rounding` to an integer of `width`,
/// as the 64-bit FRT value.
///
/// The function sign-extends a 32-bit integer into the high word. The
/// architecture leaves that word undefined; the ppu_fp_convert_modes and
/// ppu_reference_vectors console captures read the sign extension. The
/// status has FR when rounding raised the magnitude, and FI when the
/// result is inexact. The bits are an integer and name no FPRF class.
///
/// [PPC-Book1 p:117 s:4.6.6] fctiw and fctiwz place the integer in `FRT[32:63]`, leave `FRT[0:31]` and FPRF undefined, and set FR and FI; fctiwz rounds toward zero.
/// [PPC-Book1 p:116 s:4.6.6] fctid and fctidz convert to a 64-bit integer the same way.
/// [PPC-Book1 p:97 s:4.4.1] an invalid convert to integer with VE=0 gives the most positive integer for a positive operand and the most negative for a negative operand or a NaN, sets VXCVI, and sets FR and FI to zero.
pub fn ppc_convert_to_integer(width: PpcInteger, rounding: Rounding, b: u64) -> PpcResult {
    let negative = b >> 63 == 1;
    let invalid = |nan: bool, snan: bool| PpcResult {
        bits: width.saturated(negative || nan),
        status: PpcStatus {
            invalid: PpcInvalid {
                convert: true,
                snan,
                ..PpcInvalid::default()
            },
            ..PpcStatus::default()
        },
    };
    let x = match unpack::<Binary64>(Policy::PowerPc, b).0 {
        Operand::NaN { quiet } => return invalid(true, !quiet),
        Operand::Infinity { .. } => return invalid(false, false),
        Operand::Zero { .. } => {
            return PpcResult {
                bits: 0,
                status: PpcStatus::default(),
            }
        }
        Operand::Finite(x) => x,
    };
    let (magnitude, inexact, incremented) = round_to_integer(rounding, x);
    let (most_positive, most_negative) = width.limits();
    let limit = if negative {
        most_negative
    } else {
        most_positive
    };
    if magnitude > limit {
        return invalid(false, false);
    }
    // The magnitude is at most 2^63, so the cast to i128 is exact.
    let value = magnitude as i128;
    let signed = if negative { -value } else { value };
    PpcResult {
        bits: signed as i64 as u64,
        status: PpcStatus {
            fr: incremented,
            fi: inexact,
            ..PpcStatus::default()
        },
    }
}

/// The magnitude of `x` rounded to an integer in `rounding`, whether it
/// was inexact, and whether rounding raised it. An operand whose scale
/// exponent is above 70 is out of every integer range and reads as
/// `u128::MAX`.
fn round_to_integer(rounding: Rounding, x: Exact) -> (u128, bool, bool) {
    let (significand, exponent) = (x.significand(), x.exponent());
    if exponent >= 0 {
        // An operand has at most 53 significant bits.
        let whole = if exponent > 70 {
            u128::MAX
        } else {
            significand << exponent
        };
        return (whole, false, false);
    }
    let shift = exponent.unsigned_abs();
    let (whole, half, rest) = if shift > 120 {
        (0, false, significand != 0)
    } else {
        (
            significand >> shift,
            (significand >> (shift - 1)) & 1 == 1,
            significand & ((1 << (shift - 1)) - 1) != 0,
        )
    };
    let inexact = half || rest;
    let up = match rounding {
        Rounding::NearestEven => half && (rest || whole & 1 == 1),
        Rounding::TowardZero => false,
        Rounding::TowardPositive => inexact && !x.negative(),
        Rounding::TowardNegative => inexact && x.negative(),
    };
    (whole + u128::from(up), inexact, up)
}

/// The 64-bit signed integer `b` as a double, rounded once in `rounding`.
///
/// [PPC-Book1 p:118 s:4.6.6] fcfid converts the 64-bit signed integer in FRB to double format, rounded by `FPSCR[RN]`, and sets FPRF, FR, FI, FX and XX.
pub fn ppc_from_integer(rounding: Rounding, b: u64) -> PpcResult {
    let value = b as i64;
    let exact = Exact::from_parts(value < 0, u128::from(value.unsigned_abs()), 0, false);
    ppc_round_disabled(PpcPrecision::Double, rounding, exact)
}

/// The double-format `b` rounded once to single precision in `rounding`.
///
/// The function returns a zero or an infinity unchanged. It quiets a NaN
/// and clears the low-order 29 fraction bits. An SNaN also sets VXSNAN.
///
/// [PPC-Book1 p:115 s:4.6.6] frsp rounds FRB to single precision by `FPSCR[RN]` and sets FPRF, FR, FI, FX, OX, UX, XX and VXSNAN.
/// [PPC-Book1 p:140 s:A.1] a QNaN gives `FRB[0:34] || 0^29`; an SNaN with VE=0 gives `FRB[0:11] || 1 || FRB[13:34] || 0^29`.
pub fn ppc_round_to_single(rounding: Rounding, b: u64) -> PpcResult {
    let status = PpcStatus {
        invalid: PpcInvalid::signaling(&[b]),
        ..PpcStatus::default()
    };
    match unpack::<Binary64>(Policy::PowerPc, b).0 {
        Operand::NaN { .. } => PpcResult {
            bits: ppc_frsp_nan(b),
            status,
        },
        Operand::Finite(x) => ppc_round_disabled(PpcPrecision::Single, rounding, x),
        Operand::Zero { .. } | Operand::Infinity { .. } => PpcResult { bits: b, status },
    }
}
