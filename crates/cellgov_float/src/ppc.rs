//! The PowerPC scalar floating-point rules that the PPE adds to the IEEE
//! rounding:
//!
//! - the FR and FI status bits;
//! - the invalid-operation causes;
//! - single precision in double format;
//! - NaN selection;
//! - the sign of an exact zero sum.
//!
//! The rounding mode and the exception enables are arguments, and the
//! status bits are a return value. The caller holds the FPSCR.

use crate::arith::add;
use crate::format::{Binary32, Binary64, Format};
use crate::operand::{unpack, Operand};
use crate::round::{round_ieee_traced, Exact, Policy, Rounding};

/// The QNaN that a disabled invalid operation generates.
///
/// [PPC-Book1 p:91 s:4.3.2] the generated QNaN has a 0 sign, an all-ones exponent and only the high-order fraction bit set: 0x7FF8_0000_0000_0000.
pub const PPC_GENERATED_NAN: u64 = 0x7FF8_0000_0000_0000;

const SIGN: u64 = 1 << 63;
const INFINITY: u64 = 0x7FF0_0000_0000_0000;
const FRACTION: u64 = (1 << 52) - 1;
const QUIET: u64 = 1 << 51;

/// The precision an operation rounds its result to. Both variants deliver
/// the result in double format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpcPrecision {
    /// Double precision.
    Double,
    /// Single precision, widened to double format.
    Single,
}

/// The FPSCR exception enable bits: VE, OE, UE, ZE and XE.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PpcEnables {
    /// VE: invalid operation.
    pub invalid: bool,
    /// OE: overflow.
    pub overflow: bool,
    /// UE: underflow.
    pub underflow: bool,
    /// ZE: zero divide.
    pub zero_divide: bool,
    /// XE: inexact. It changes no result.
    pub inexact: bool,
}

/// The FPSCR fields that an operation reads.
///
/// A [`Rounding`] converts to a control with every exception disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PpcControl {
    /// The rounding mode, from `FPSCR[RN]`.
    pub rounding: Rounding,
    /// The exception enables.
    pub enables: PpcEnables,
}

impl From<Rounding> for PpcControl {
    fn from(rounding: Rounding) -> PpcControl {
        PpcControl {
            rounding,
            enables: PpcEnables::default(),
        }
    }
}

/// An enabled exception that leaves the target register unchanged. The
/// caller writes the status and keeps the old FRT and FPRF.
///
/// [PPC-Book1 p:97 s:4.4.1] an enabled invalid operation leaves the target unchanged.
/// [PPC-Book1 p:98 s:4.4.2] an enabled zero divide leaves the target unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PpcEnabled {
    /// VE = 1 and an invalid operation occurred.
    #[error("enabled invalid operation exception (VE = 1)")]
    Invalid,
    /// ZE = 1 and a zero divide occurred.
    #[error("enabled zero divide exception (ZE = 1)")]
    ZeroDivide,
}

/// The invalid-operation causes, one per FPSCR VX bit an operation can set.
///
/// [PPC-Book1 p:97 s:4.4.1] the invalid operations and the VX bit each sets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PpcInvalid {
    /// VXSNAN: an operand is a signaling NaN.
    pub snan: bool,
    /// VXISI: a magnitude subtraction of infinities.
    pub inf_minus_inf: bool,
    /// VXIDI: infinity divided by infinity.
    pub inf_div_inf: bool,
    /// VXZDZ: zero divided by zero.
    pub zero_div_zero: bool,
    /// VXIMZ: infinity times zero.
    pub inf_times_zero: bool,
    /// VXVC: an ordered comparison involving a NaN.
    pub compare: bool,
    /// VXSQRT: the square root of a negative nonzero number.
    pub sqrt: bool,
    /// VXCVI: an integer conversion of a NaN, an infinity or a number too
    /// large for the target.
    pub convert: bool,
}

impl PpcInvalid {
    /// Whether any cause is set.
    pub fn any(&self) -> bool {
        self.snan
            || self.inf_minus_inf
            || self.inf_div_inf
            || self.zero_div_zero
            || self.inf_times_zero
            || self.compare
            || self.sqrt
            || self.convert
    }

    /// The union of two cause sets.
    pub fn or(self, other: PpcInvalid) -> PpcInvalid {
        PpcInvalid {
            snan: self.snan || other.snan,
            inf_minus_inf: self.inf_minus_inf || other.inf_minus_inf,
            inf_div_inf: self.inf_div_inf || other.inf_div_inf,
            zero_div_zero: self.zero_div_zero || other.zero_div_zero,
            inf_times_zero: self.inf_times_zero || other.inf_times_zero,
            compare: self.compare || other.compare,
            sqrt: self.sqrt || other.sqrt,
            convert: self.convert || other.convert,
        }
    }

    /// VXSNAN when any of the double-format `operands` is a signaling NaN.
    pub fn signaling(operands: &[u64]) -> PpcInvalid {
        PpcInvalid {
            snan: operands.iter().any(|&b| ppc_is_snan(b)),
            ..PpcInvalid::default()
        }
    }
}

/// The status bits one operation reports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PpcStatus {
    /// FR: rounding incremented the fraction.
    pub fr: bool,
    /// FI: the result is inexact, or a disabled overflow occurred.
    pub fi: bool,
    /// OX: overflow.
    pub ox: bool,
    /// UX: underflow; with UE = 0, a tiny and inexact result.
    pub ux: bool,
    /// ZX: zero divide.
    pub zx: bool,
    /// The invalid-operation causes.
    pub invalid: PpcInvalid,
}

impl PpcStatus {
    /// The refusal for an enabled invalid operation or zero divide in this
    /// status, under `enables`.
    pub fn refusal(&self, enables: PpcEnables) -> Option<PpcEnabled> {
        if enables.invalid && self.invalid.any() {
            Some(PpcEnabled::Invalid)
        } else if enables.zero_divide && self.zx {
            Some(PpcEnabled::ZeroDivide)
        } else {
            None
        }
    }

    /// Whether `enables` makes the result the scaled result of an overflow or underflow.
    ///
    /// A scaled single result can lie outside the single range, so its
    /// FPRF class is a double class. The ppu_fp_enabled_results console
    /// capture reads FPRF +normal for the frsp of 2^-1074 under UE = 1.
    /// That scaled result is 2^-882.
    pub fn scaled(&self, enables: PpcEnables) -> bool {
        (enables.overflow && self.ox) || (enables.underflow && self.ux)
    }
}

/// A double-format result and the status bits its operation reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PpcResult {
    /// The result, in double format.
    pub bits: u64,
    /// The status bits.
    pub status: PpcStatus,
}

/// The exponent adjustment of an enabled overflow or underflow.
///
/// [PPC-Book1 p:98 s:4.4.3] with OE = 1 the exponent is reduced by 1536 for double-precision operations and by 192 for single-precision operations and frsp.
/// [PPC-Book1 p:99 s:4.4.4] with UE = 1 the exponent is increased by the same amounts.
pub(crate) fn exponent_adjustment(precision: PpcPrecision) -> i32 {
    match precision {
        PpcPrecision::Double => 1536,
        PpcPrecision::Single => 192,
    }
}

/// The format that a scaled single result rounds to.
///
/// The format has a 24-bit significand and the double exponent range. Its
/// encoding is the double format without the low-order 29 fraction bits.
/// The ppu_fp_enabled_results console capture reads these results:
///
/// - 2^832 for the frsp of the largest double under OE = 1;
/// - 2^-882 for the frsp of 2^-1074 under UE = 1.
///
/// Both results lie outside the single range.
struct SingleSignificand;

impl Format for SingleSignificand {
    const EXP_BITS: u32 = 11;
    const FRAC_BITS: u32 = 23;
}

/// Rounds `exact` to `precision` under `control` and delivers the result in double format.
///
/// An enabled overflow or underflow adjusts the exponent of the exact
/// value, then rounds it once. The adjusted value lies inside the range of
/// the format, so this rounding cannot overflow or underflow again. FR and
/// FI come from this rounding, and the result sets OX or UX.
///
/// [PPC-Book1 p:94 s:4.3.6] FR is set when rounding increments the fraction, FI when the result is inexact.
/// [PPC-Book1 p:98 s:4.4.3] a disabled overflow delivers infinity or the largest finite number by mode and sign, sets FI and leaves FR undefined; the ppu_reference_vectors console capture reads FR = 1 after every overflow, in every mode and both widths.
/// [PPC-Book1 p:98 s:4.4.4] tininess is detected before rounding; with UE = 0 the value is denormalized and rounded, and UX is set only when the result is also inexact.
/// [PPC-Book1 p:98 s:4.4.4] with UE = 1 an underflow is any tiny result, exact or not.
/// [PPC-Book1 p:99 s:4.4.5] with OE = 1 or UE = 1, XX is set only when the rounded and the intermediate significands differ.
/// [PPC-Book1 p:93 s:4.3.5.1] a single-precision result lies in single range and is stored in double format with the low-order 29 fraction bits zero.
pub fn ppc_round(
    precision: PpcPrecision,
    control: impl Into<PpcControl>,
    exact: Exact,
) -> PpcResult {
    let PpcControl { rounding, enables } = control.into();
    let (result, tiny) = match precision {
        PpcPrecision::Double => round_disabled_in::<Binary64>(rounding, exact),
        PpcPrecision::Single => {
            let (r, tiny) = round_disabled_in::<Binary32>(rounding, exact);
            let bits = widen_single(r.bits);
            (PpcResult { bits, ..r }, tiny)
        }
    };
    let status = if enables.overflow && result.status.ox {
        PpcStatus {
            ox: true,
            ..PpcStatus::default()
        }
    } else if enables.underflow && tiny {
        PpcStatus {
            ux: true,
            ..PpcStatus::default()
        }
    } else {
        return result;
    };
    let adjustment = exponent_adjustment(precision);
    let shift = if status.ox { -adjustment } else { adjustment };
    let adjusted = Exact::from_parts(
        exact.negative,
        exact.significand,
        exact.exponent + shift,
        exact.sticky,
    );
    round_adjusted(precision, rounding, adjusted, status)
}

/// Rounds the adjusted `exact` to `precision` and adds FR and FI to `status`.
fn round_adjusted(
    precision: PpcPrecision,
    rounding: Rounding,
    exact: Exact,
    status: PpcStatus,
) -> PpcResult {
    let (traced, bits) = match precision {
        PpcPrecision::Double => {
            let traced = round_ieee_traced::<Binary64>(rounding, exact);
            let bits = traced.packed.bits;
            (traced, bits)
        }
        PpcPrecision::Single => {
            let traced = round_ieee_traced::<SingleSignificand>(rounding, exact);
            let packed = traced.packed.bits;
            let sign = packed >> SingleSignificand::SIGN_SHIFT << 63;
            let magnitude = packed & ((1 << SingleSignificand::SIGN_SHIFT) - 1);
            (traced, sign | magnitude << 29)
        }
    };
    PpcResult {
        bits,
        status: PpcStatus {
            fr: traced.incremented,
            fi: traced.packed.flags.inexact,
            ..status
        },
    }
}

/// `exact` rounded into format `F` with every exception disabled, in
/// `F`'s own encoding, and whether the value is tiny.
pub(crate) fn round_disabled_in<F: Format>(rounding: Rounding, exact: Exact) -> (PpcResult, bool) {
    let traced = round_ieee_traced::<F>(rounding, exact);
    let flags = traced.packed.flags;
    let result = PpcResult {
        bits: traced.packed.bits,
        status: PpcStatus {
            fr: traced.incremented || flags.overflow,
            fi: flags.inexact,
            ox: flags.overflow,
            ux: flags.underflow,
            ..PpcStatus::default()
        },
    };
    (result, traced.tiny)
}

/// The binary32 word `bits` in double format. A rounded single result is
/// never a NaN; the NaN arm keeps the fraction's high bits.
fn widen_single(bits: u64) -> u64 {
    let sign = (bits >> 31 & 1) << 63;
    match unpack::<Binary32>(Policy::PowerPc, bits).0 {
        Operand::Zero { .. } => sign,
        Operand::Infinity { .. } => sign | INFINITY,
        Operand::NaN { .. } => sign | INFINITY | (bits & 0x7F_FFFF) << 29,
        Operand::Finite(x) => {
            round_ieee_traced::<Binary64>(Rounding::NearestEven, x)
                .packed
                .bits
        }
    }
}

/// The exact sum `a + b`, with the PowerPC sign for an exact zero sum.
///
/// When the operand signs agree, a zero sum takes that sign. Returns
/// `None` when an operand is too wide for [`add`].
///
/// [PPC-Book1 p:92 s:4.3.3] the exact zero sum of opposite-signed operands is positive in every mode except round toward -infinity, where it is negative.
pub fn ppc_sum(rounding: Rounding, a: Exact, b: Exact) -> Option<Exact> {
    add(a, b).map(|s| signed_zero_sum(rounding, a, b, s))
}

/// `s`, the sum of `a` and `b`, with the PowerPC sign when it is zero.
pub(crate) fn signed_zero_sum(rounding: Rounding, a: Exact, b: Exact, s: Exact) -> Exact {
    if s.significand() != 0 {
        return s;
    }
    let negative = if a.negative() == b.negative() {
        a.negative()
    } else {
        rounding == Rounding::TowardNegative
    };
    Exact::from_parts(negative, 0, s.exponent(), false)
}

/// Whether the double-format `bits` is a NaN.
pub fn ppc_is_nan(bits: u64) -> bool {
    bits & INFINITY == INFINITY && bits & FRACTION != 0
}

/// Whether the double-format `bits` is a signaling NaN.
///
/// [PPC-Book1 p:91 s:4.3.2] a NaN whose high-order fraction bit is 0 is a signaling NaN.
pub fn ppc_is_snan(bits: u64) -> bool {
    ppc_is_nan(bits) && bits & QUIET == 0
}

/// The first NaN in the order FRA, FRB, FRC, with its quiet bit set.
///
/// A signaling NaN and a quiet NaN have the same rank. An operand that the
/// operation does not read is `None`. Returns `None` when no operand is a
/// NaN.
///
/// [PPC-Book1 p:91 s:4.3.2] a NaN in FRA is the result, else one in FRB, else one in FRC, else the generated QNaN; the result has its high-order fraction bit set.
pub fn ppc_nan_result(fra: Option<u64>, frb: Option<u64>, frc: Option<u64>) -> Option<u64> {
    [fra, frb, frc]
        .into_iter()
        .flatten()
        .find(|&b| ppc_is_nan(b))
        .map(|b| b | QUIET)
}

/// The `frsp` result for a NaN `frb`: quieted, with the low-order 29 bits
/// cleared.
///
/// [PPC-Book1 p:91 s:4.3.2] frsp stores (FRB) bits 0:34 followed by 29 zero bits.
pub fn ppc_frsp_nan(frb: u64) -> u64 {
    (frb | QUIET) & !((1 << 29) - 1)
}

fn is_infinity(bits: u64) -> bool {
    bits & !SIGN == INFINITY
}

fn is_zero(bits: u64) -> bool {
    bits & !SIGN == 0
}

/// VXISI for `a + b`: infinities of opposite sign. A subtraction passes
/// `b` negated.
///
/// [PPC-Book1 p:97 s:4.4.1] the magnitude subtraction of infinities is invalid.
pub fn ppc_invalid_sum(a: u64, b: u64) -> PpcInvalid {
    PpcInvalid {
        inf_minus_inf: is_infinity(a) && is_infinity(b) && (a ^ b) & SIGN != 0,
        ..PpcInvalid::default()
    }
}

/// VXIMZ for `a x c`: an infinity times a zero.
///
/// [PPC-Book1 p:97 s:4.4.1] the multiplication of infinity by zero is invalid.
pub fn ppc_invalid_product(a: u64, c: u64) -> PpcInvalid {
    PpcInvalid {
        inf_times_zero: (is_infinity(a) && is_zero(c)) || (is_zero(a) && is_infinity(c)),
        ..PpcInvalid::default()
    }
}

/// VXIDI and VXZDZ for `a / b`.
///
/// [PPC-Book1 p:97 s:4.4.1] infinity divided by infinity and zero divided by zero are invalid.
pub fn ppc_invalid_quotient(a: u64, b: u64) -> PpcInvalid {
    PpcInvalid {
        inf_div_inf: is_infinity(a) && is_infinity(b),
        zero_div_zero: is_zero(a) && is_zero(b),
        ..PpcInvalid::default()
    }
}

/// VXSQRT for the square root of `b`: -infinity or a negative nonzero
/// number.
///
/// [PPC-Book1 p:97 s:4.4.1] the square root of a negative nonzero number is invalid.
pub fn ppc_invalid_sqrt(b: u64) -> PpcInvalid {
    PpcInvalid {
        sqrt: b & SIGN != 0 && !is_zero(b) && !ppc_is_nan(b),
        ..PpcInvalid::default()
    }
}
