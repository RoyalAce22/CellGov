//! The PowerPC scalar add, subtract, multiply, divide, square root and
//! multiply-add, with every exception disabled. The module owns these
//! steps:
//!
//! - NaN selection;
//! - the invalid-operation causes;
//! - the single-precision operand range;
//! - one rounding in the caller's mode.
//!
//! The result is the one a disabled exception gives. The status reports
//! each exception that occurred, so a caller can tell when an enable bit
//! would change the result.

use crate::arith::{product, quotient, square_root, sum};
use crate::format::Binary64;
use crate::operand::{unpack, Operand};
use crate::ppc::{
    ppc_invalid_product, ppc_invalid_quotient, ppc_invalid_sqrt, ppc_invalid_sum, ppc_is_nan,
    ppc_nan_result, ppc_round_disabled, signed_zero_sum, PpcInvalid, PpcPrecision, PpcResult,
    PpcStatus, PPC_GENERATED_NAN,
};
use crate::round::{Exact, Policy, Rounding};

const SIGN: u64 = 1 << 63;
const INFINITY: u64 = 0x7FF0_0000_0000_0000;

/// The lowest and highest unbiased exponent of a single-precision
/// operand the PPE computes on: the smallest single denormal and the
/// largest single binade.
const SINGLE_OPERAND_EXPONENTS: std::ops::RangeInclusive<i32> = -149..=127;

/// The double-format `a + b`, rounded to `precision` in `rounding`.
///
/// [PPC-Book1 p:111 s:4.6.5.1] fadd and fadds: FRT <- FRA + FRB, rounded to the target precision by `FPSCR[RN]`.
pub fn ppc_add(precision: PpcPrecision, rounding: Rounding, a: u64, b: u64) -> PpcResult {
    arith(precision, rounding, Combine::Sum, (a, b), (a, b))
}

/// The double-format `a - b`, rounded to `precision` in `rounding`. A NaN
/// in `b` keeps its sign.
///
/// [PPC-Book1 p:111 s:4.6.5.1] fsub and fsubs: FRT <- FRA - FRB.
pub fn ppc_sub(precision: PpcPrecision, rounding: Rounding, a: u64, b: u64) -> PpcResult {
    arith(precision, rounding, Combine::Sum, (a, b), (a, b ^ SIGN))
}

/// The double-format `a x c`, rounded to `precision` in `rounding`.
///
/// [PPC-Book1 p:112 s:4.6.5.1] fmul and fmuls: FRT <- FRA x FRC; FRB is not an operand.
pub fn ppc_mul(precision: PpcPrecision, rounding: Rounding, a: u64, c: u64) -> PpcResult {
    arith(precision, rounding, Combine::Product, (a, c), (a, c))
}

/// The double-format `a / b`, rounded to `precision` in `rounding`.
///
/// A finite nonzero `a` over a zero `b` is a zero divide: the result is
/// an infinity with the exclusive OR of the signs, and the status has
/// ZX.
///
/// [PPC-Book1 p:112 s:4.6.5.1] fdiv and fdivs: FRT <- FRA / FRB, rounded to the target precision by `FPSCR[RN]`.
/// [PPC-Book1 p:98 s:4.4.2] a disabled zero divide gives an infinity signed by the operand signs, and sets FR and FI to zero.
/// [PPC-Book1 p:97 s:4.4.1] infinity over infinity and zero over zero are invalid.
pub fn ppc_div(precision: PpcPrecision, rounding: Rounding, a: u64, b: u64) -> PpcResult {
    let invalid = PpcInvalid::signaling(&[a, b]).or(ppc_invalid_quotient(a, b));
    let status = PpcStatus {
        invalid,
        ..PpcStatus::default()
    };
    if let Some(nan) = ppc_nan_result(Some(a), Some(b), None) {
        return PpcResult { bits: nan, status };
    }
    if invalid.any() {
        return PpcResult {
            bits: PPC_GENERATED_NAN,
            status,
        };
    }
    let sign = (a ^ b) & SIGN;
    let (zero, infinite) = (|v: u64| v & !SIGN == 0, |v: u64| v & !SIGN == INFINITY);
    if zero(b) && !infinite(a) && !zero(a) {
        return PpcResult {
            bits: sign | INFINITY,
            status: PpcStatus { zx: true, ..status },
        };
    }
    if infinite(a) {
        return PpcResult {
            bits: sign | INFINITY,
            status,
        };
    }
    if infinite(b) || zero(a) {
        return PpcResult { bits: sign, status };
    }
    ppc_round_disabled(precision, rounding, quotient(finite(a), finite(b)))
}

/// The square root of the double-format `b`, rounded to `precision` in
/// `rounding`.
///
/// The root of a zero is that zero, and the root of +infinity is
/// +infinity.
///
/// [PPC-Book1 p:125 s:5.2.1.1] fsqrt and fsqrts: FRT <- the square root of FRB, rounded to the target precision by `FPSCR[RN]`.
/// [PPC-Book1 p:92 s:4.3.3] the square root of -0 is -0.
/// [PPC-Book1 p:97 s:4.4.1] the square root of a negative nonzero number, -infinity included, is invalid.
pub fn ppc_sqrt(precision: PpcPrecision, rounding: Rounding, b: u64) -> PpcResult {
    let invalid = PpcInvalid::signaling(&[b]).or(ppc_invalid_sqrt(b));
    let status = PpcStatus {
        invalid,
        ..PpcStatus::default()
    };
    if let Some(nan) = ppc_nan_result(None, Some(b), None) {
        return PpcResult { bits: nan, status };
    }
    if invalid.any() {
        return PpcResult {
            bits: PPC_GENERATED_NAN,
            status,
        };
    }
    if b & !SIGN == 0 || b == INFINITY {
        return PpcResult { bits: b, status };
    }
    ppc_round_disabled(precision, rounding, square_root(finite(b)))
}

/// The double-format `a x c + b`, rounded once to `precision` in
/// `rounding`.
///
/// [PPC-Book1 p:113 s:4.6.5.2] fmadd and fmadds: FRT <- (FRA x FRC) + FRB, with one rounding.
pub fn ppc_madd(precision: PpcPrecision, rounding: Rounding, a: u64, b: u64, c: u64) -> PpcResult {
    multiply_add(precision, rounding, (a, b, c), false)
}

/// The double-format `a x c - b`, rounded once to `precision` in
/// `rounding`. A NaN in `b` keeps its sign.
///
/// [PPC-Book1 p:113 s:4.6.5.2] fmsub and fmsubs: FRT <- (FRA x FRC) - FRB.
pub fn ppc_msub(precision: PpcPrecision, rounding: Rounding, a: u64, b: u64, c: u64) -> PpcResult {
    multiply_add(precision, rounding, (a, b, c), true)
}

/// [`ppc_madd`] with the rounded result negated. A NaN result keeps its
/// sign.
///
/// [PPC-Book1 p:114 s:4.6.5.2] fnmadd and fnmadds: FRT <- -((FRA x FRC) + FRB); QNaNs propagate with no effect on their sign bit.
/// [PPC-Book1 p:114 s:4.6.5.2] A generated QNaN has sign 0; an SNaN quieted in its place keeps the SNaN's sign.
pub fn ppc_nmadd(precision: PpcPrecision, rounding: Rounding, a: u64, b: u64, c: u64) -> PpcResult {
    negate_unless_nan(ppc_madd(precision, rounding, a, b, c))
}

/// [`ppc_msub`] with the rounded result negated. A NaN result keeps its
/// sign.
///
/// [PPC-Book1 p:114 s:4.6.5.2] fnmsub and fnmsubs: FRT <- -((FRA x FRC) - FRB).
pub fn ppc_nmsub(precision: PpcPrecision, rounding: Rounding, a: u64, b: u64, c: u64) -> PpcResult {
    negate_unless_nan(ppc_msub(precision, rounding, a, b, c))
}

/// The precision a divide or square root of `operands` rounds to when its
/// instruction names `precision`.
///
/// A single-precision instruction rounds to double precision when an
/// operand lies outside the single exponents. The ppu_fp_divsqrt_modes
/// console capture shows that fdivs and fsqrts on such an operand store
/// the double-precision result with double-precision status and FPRF:
///
/// - 2^128 / 3 rounds to a double;
/// - 1 / 2^-1074 overflows the double range;
/// - 1 / 2^128 is a normal 2^-128.
///
/// [PPC-Book1 p:93 s:4.3.5.1] a single-precision operation on an operand that is not representable in single format gives undefined results.
pub fn ppc_div_sqrt_precision(precision: PpcPrecision, operands: &[u64]) -> PpcPrecision {
    if operands.iter().any(|&v| ppc_single_operand_out_of_range(v)) {
        PpcPrecision::Double
    } else {
        precision
    }
}

/// Whether a single-precision arithmetic operand is a finite nonzero value
/// outside the exponents the PPE computes on: 2^128 or above, or below the
/// smallest single denormal.
///
/// The architecture leaves the result undefined for any operand single
/// format cannot hold. The ppu_fp_addmul_modes console capture separates
/// two cases:
///
/// - the PPE computes an operand with fraction bits below single
///   precision at its double value, when its exponent is in range;
/// - an operand this function names makes fadds, fsubs and fmuls give
///   the generated QNaN with no exception bit, unless the other operand is
///   a NaN or an infinity.
///
/// [PPC-Book1 p:93 s:4.3.5.1] a single-precision operation on an operand that is not representable in single format gives undefined results.
pub(crate) fn ppc_single_operand_out_of_range(bits: u64) -> bool {
    let biased = (bits >> 52 & 0x7FF) as i32;
    if bits & !SIGN == 0 || biased == 0x7FF {
        return false;
    }
    biased == 0 || !SINGLE_OPERAND_EXPONENTS.contains(&(biased - 1023))
}

/// How the two operands combine.
#[derive(Clone, Copy)]
enum Combine {
    Sum,
    Product,
}

/// `combine` of the operands `(x, y)`, whose register values are `raw`.
///
/// The steps run in this order:
///
/// 1. a NaN operand, chosen from `raw`;
/// 2. an invalid operation, which gives the generated QNaN;
/// 3. an infinity operand;
/// 4. a single-precision operand out of range;
/// 5. the exact result, rounded once.
///
/// [PPC-Book1 p:91 s:4.3.2] a NaN operand is the result; a disabled invalid operation gives the generated QNaN.
/// [PPC-Book1 p:97 s:4.4.1] an SNaN operand, infinity minus infinity and infinity times zero are invalid.
fn arith(
    precision: PpcPrecision,
    rounding: Rounding,
    combine: Combine,
    raw: (u64, u64),
    (x, y): (u64, u64),
) -> PpcResult {
    let cause = match combine {
        Combine::Sum => ppc_invalid_sum(x, y),
        Combine::Product => ppc_invalid_product(x, y),
    };
    let invalid = PpcInvalid::signaling(&[raw.0, raw.1]).or(cause);
    let status = PpcStatus {
        invalid,
        ..PpcStatus::default()
    };
    if let Some(nan) = ppc_nan_result(Some(raw.0), Some(raw.1), None) {
        return PpcResult { bits: nan, status };
    }
    if invalid.any() {
        return PpcResult {
            bits: PPC_GENERATED_NAN,
            status,
        };
    }
    if let Some(bits) = infinite_result(combine, x, y) {
        return PpcResult { bits, status };
    }
    if precision == PpcPrecision::Single
        && (ppc_single_operand_out_of_range(x) || ppc_single_operand_out_of_range(y))
    {
        return PpcResult {
            bits: PPC_GENERATED_NAN,
            status,
        };
    }
    let (ex, ey) = (finite(x), finite(y));
    let value = match combine {
        Combine::Sum => signed_zero_sum(rounding, ex, ey, sum(ex, ey)),
        Combine::Product => product(ex, ey),
    };
    ppc_round_disabled(precision, rounding, value)
}

/// The infinity `combine` gives when an operand is an infinity, or `None`.
/// [`arith`] rejects an invalid operation first, so an infinite sum has
/// one sign and an infinite product has a nonzero factor.
fn infinite_result(combine: Combine, x: u64, y: u64) -> Option<u64> {
    let infinite = |v: u64| v & !SIGN == INFINITY;
    match combine {
        Combine::Sum if infinite(x) => Some(x),
        Combine::Sum if infinite(y) => Some(y),
        Combine::Product if infinite(x) || infinite(y) => Some(INFINITY | ((x ^ y) & SIGN)),
        _ => None,
    }
}

/// `a x c + b`, or `a x c - b` when `subtract`, rounded once.
///
/// The steps run in the order of [`arith`]. The single-precision range
/// check covers all three operands. The `ppu_fp_fma_modes` console capture
/// shows that fmadds gives the generated QNaN, with no exception bit, for
/// an operand out of range in any position. This holds when the product is
/// in range. A NaN or infinity operand gives its own result first.
///
/// [PPC-Book1 p:92 s:4.3.3] a multiply-add applies the sign rules to the product first, then to the sum.
/// [PPC-Book1 p:113 s:4.6.5.2] the invalid-operation bits are set as if the multiply and the add ran as two instructions.
/// [PPC-Book1 p:97 s:4.4.1] the invalid operations: an SNaN operand, infinity minus infinity, infinity times zero; a disabled invalid operation sets FR and FI to zero.
fn multiply_add(
    precision: PpcPrecision,
    rounding: Rounding,
    (a, b, c): (u64, u64, u64),
    subtract: bool,
) -> PpcResult {
    let addend = if subtract { b ^ SIGN } else { b };
    let infinite = infinite_product(a, c);
    let invalid = PpcInvalid::signaling(&[a, b, c])
        .or(ppc_invalid_product(a, c))
        .or(ppc_invalid_sum(infinite.unwrap_or(0), addend));
    let status = PpcStatus {
        invalid,
        ..PpcStatus::default()
    };
    if let Some(nan) = ppc_nan_result(Some(a), Some(b), Some(c)) {
        return PpcResult { bits: nan, status };
    }
    if invalid.any() {
        return PpcResult {
            bits: PPC_GENERATED_NAN,
            status,
        };
    }
    if let Some(bits) = infinite.or((addend & !SIGN == INFINITY).then_some(addend)) {
        return PpcResult { bits, status };
    }
    if precision == PpcPrecision::Single
        && [a, b, c].into_iter().any(ppc_single_operand_out_of_range)
    {
        return PpcResult {
            bits: PPC_GENERATED_NAN,
            status,
        };
    }
    let (p, e) = (product(finite(a), finite(c)), finite(addend));
    ppc_round_disabled(
        precision,
        rounding,
        signed_zero_sum(rounding, p, e, sum(p, e)),
    )
}

/// The signed infinity `a x c` gives, or `None` when the product is not
/// an infinity.
fn infinite_product(a: u64, c: u64) -> Option<u64> {
    let infinite = |v: u64| v & !SIGN == INFINITY;
    let zero_or_nan = |v: u64| v & !SIGN == 0 || ppc_is_nan(v);
    let product = (infinite(a) && !zero_or_nan(c)) || (infinite(c) && !zero_or_nan(a));
    product.then_some(INFINITY | ((a ^ c) & SIGN))
}

/// `result` with its sign flipped, unless it is a NaN.
fn negate_unless_nan(result: PpcResult) -> PpcResult {
    if ppc_is_nan(result.bits) {
        return result;
    }
    PpcResult {
        bits: result.bits ^ SIGN,
        ..result
    }
}

/// The exact value of a finite double-format operand.
fn finite(bits: u64) -> Exact {
    match unpack::<Binary64>(Policy::PowerPc, bits).0 {
        Operand::Finite(x) => x,
        _ => Exact::from_parts(bits & SIGN != 0, 0, 0, false),
    }
}
