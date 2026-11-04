//! The vector single-precision rules: binary32 under [`Policy::PowerPc`],
//! with the non-Java mode that `VSCR[NJ]` selects.
//!
//! The vector unit keeps no floating-point status, so these functions
//! return bits only.

use crate::arith::{product, sum};
use crate::format::{Binary32, Format};
use crate::operand::{unpack, Operand};
use crate::round::{binade, round_pack, Exact, Policy, Rounding};
use std::cmp::Ordering;

/// The vector floating-point mode.
///
/// [AltiVec-PEM p:3-11 s:3.2.1] `VSCR[NJ]` = 0 selects Java mode and `VSCR[NJ]` = 1 the non-Java mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmxMode {
    /// Denormal operands and results (`VSCR[NJ]` = 0).
    Java,
    /// Denormal operands and tiny results replaced by a zero of their
    /// sign (`VSCR[NJ]` = 1).
    NonJava,
}

impl VmxMode {
    /// The mode `VSCR[NJ]` = `nj` selects.
    pub fn from_nj(nj: bool) -> VmxMode {
        if nj {
            VmxMode::NonJava
        } else {
            VmxMode::Java
        }
    }
}

/// The quiet NaN an invalid operation produces.
///
/// [AltiVec-PEM p:3-13 s:3.2.4.2] the result of an invalid operation is the QNaN 0x7FC0_0000.
pub const VMX_DEFAULT_NAN: u32 = 0x7FC0_0000;

/// Decodes the binary32 word `bits` in `mode`.
pub fn vmx_unpack(mode: VmxMode, bits: u32) -> Operand {
    unpack_in::<Binary32>(mode, u64::from(bits))
}

/// Decodes `bits` in format `F`: non-Java mode reads a denormal as a
/// zero of its sign.
///
/// [AltiVec-PEM p:3-11 s:3.2.1] in non-Java mode, denormalized input operands are flushed to the correctly signed zero before use.
pub(crate) fn unpack_in<F: Format>(mode: VmxMode, bits: u64) -> Operand {
    let (operand, _) = unpack::<F>(Policy::PowerPc, bits);
    let denormal = (bits >> F::FRAC_BITS) as u32 & F::EXP_MAX == 0;
    match operand {
        Operand::Finite(x) if mode == VmxMode::NonJava && denormal => Operand::Zero {
            negative: x.negative(),
        },
        other => other,
    }
}

/// The result an operation takes from its NaN operands, given in vA, vB,
/// vC order: the first NaN, quieted. `None` when no operand is a NaN.
///
/// [AltiVec-PEM p:3-15 s:3.2.5.1] with several NaN operands the precedence is vA, then vB, then vC.
/// [AltiVec-PEM p:3-15 s:3.2.5.2] an SNaN input gives an exact copy of it with the fraction's msb set.
/// [AltiVec-PEM p:3-15 s:3.2.5.3] a QNaN input is propagated with all its information.
pub fn vmx_nan(operands: &[u32]) -> Option<u32> {
    operands
        .iter()
        .find(|&&x| x & 0x7F80_0000 == 0x7F80_0000 && x & 0x007F_FFFF != 0)
        .map(|&x| x | 0x0040_0000)
}

/// The binary32 word `bits` as `mode` reads it: a denormal becomes a
/// zero of its sign in non-Java mode, and every other word stays as it is.
///
/// [AltiVec-PEM p:3-11 s:3.2.1] non-Java mode flushes a denormalized operand to the correctly signed zero before use; the ppu_vfp_maxmin console capture shows vmaxfp and vminfp writing that zero.
pub fn vmx_flush(mode: VmxMode, bits: u32) -> u32 {
    match vmx_unpack(mode, bits) {
        Operand::Zero { negative } => u32::from(negative) << 31,
        _ => bits,
    }
}

/// The order of the binary32 words `a` and `b` in `mode`, or `None`
/// when either is a NaN. +0 and -0 are equal, and non-Java mode compares
/// a denormal as a zero of its sign.
///
/// [AltiVec-PEM p:3-11 s:3.2.1] NaNs compare unordered with everything; non-Java mode flushes a denormalized operand before use.
pub fn vmx_compare(mode: VmxMode, a: u32, b: u32) -> Option<Ordering> {
    let key = |bits: u32| match vmx_unpack(mode, bits) {
        Operand::NaN { .. } => None,
        Operand::Zero { .. } => Some(0),
        // A finite or infinite binary32 orders by its magnitude bits.
        _ => {
            let magnitude = i64::from(bits & 0x7FFF_FFFF);
            Some(if bits >> 31 == 1 {
                -magnitude
            } else {
                magnitude
            })
        }
    };
    Some(key(a)?.cmp(&key(b)?))
}

/// `a + b` on binary32 words in `mode`, rounded to nearest.
///
/// [AltiVec-PEM p:3-12 s:3.2.3] every vector float arithmetic instruction rounds to nearest.
/// [AltiVec-PEM p:3-12 s:3.2.2] operations on infinities follow IEEE.
pub fn vmx_add(mode: VmxMode, a: u32, b: u32) -> u32 {
    vmx_nan(&[a, b]).unwrap_or_else(|| add_in(mode, a, b))
}

/// `a - b` on binary32 words in `mode`, rounded to nearest. A NaN in `b`
/// keeps its sign.
pub fn vmx_sub(mode: VmxMode, a: u32, b: u32) -> u32 {
    vmx_nan(&[a, b]).unwrap_or_else(|| add_in(mode, a, b ^ 0x8000_0000))
}

/// `a * c + b` on binary32 words in `mode`, rounded once to nearest. NaN
/// precedence is `a`, `b`, `c`: the vA, vB, vC order of `vmaddfp`.
///
/// [AltiVec-PEM p:6-71 s:6.2] vmaddfp rounds the exact (vA x vC) + vB once; vB = -0.0 makes it a multiply.
pub fn vmx_madd(mode: VmxMode, a: u32, b: u32, c: u32) -> u32 {
    vmx_nan(&[a, b, c]).unwrap_or_else(|| madd_in(mode, a, b, c))
}

/// `-(a * c - b)` on binary32 words in `mode`, rounded once to nearest.
/// A NaN result keeps its sign.
///
/// [AltiVec-PEM p:6-109 s:6.2] vnmsubfp inverts the sign of the rounded difference, and a QNaN result is not negated.
pub fn vmx_nmsub(mode: VmxMode, a: u32, b: u32, c: u32) -> u32 {
    vmx_nan(&[a, b, c]).unwrap_or_else(|| {
        let difference = madd_in(mode, a, b ^ 0x8000_0000, c);
        if vmx_nan(&[difference]).is_some() {
            difference
        } else {
            difference ^ 0x8000_0000
        }
    })
}

/// Rounds the binary32 word `b` to an integer value by `rounding` in `mode`.
///
/// An integer, an infinity or a zero keeps its value and sign. A NaN
/// gives its quiet copy.
///
/// [AltiVec-PEM p:3-10 s:3.2] the round-to-integer instructions use their own rounding modes.
/// [AltiVec-PEM p:3-11 s:3.2.1] non-Java mode reads a denormal operand as a zero of its sign.
pub fn vmx_round_integer(mode: VmxMode, rounding: Rounding, b: u32) -> u32 {
    if let Some(nan) = vmx_nan(&[b]) {
        return nan;
    }
    let x = match vmx_unpack(mode, b) {
        Operand::Finite(x) if x.exponent() < 0 => x,
        Operand::Zero { negative } => return u32::from(negative) << 31,
        _ => return b,
    };
    let shift = x.exponent().unsigned_abs();
    // A shift past the significand's width puts every bit below the binary
    // point and the whole fraction below one half.
    let width = u128::BITS - x.significand().leading_zeros();
    let (whole, fraction, half) = if shift > width {
        (0, x.significand(), u128::MAX)
    } else {
        let mask = (1u128 << shift) - 1;
        (
            x.significand() >> shift,
            x.significand() & mask,
            1u128 << (shift - 1),
        )
    };
    let up = match rounding {
        Rounding::NearestEven => fraction > half || (fraction == half && whole & 1 == 1),
        Rounding::TowardZero => false,
        Rounding::TowardPositive => fraction != 0 && !x.negative(),
        Rounding::TowardNegative => fraction != 0 && x.negative(),
    };
    let magnitude = whole + u128::from(up);
    if magnitude == 0 {
        return u32::from(x.negative()) << 31;
    }
    vmx_round(
        mode,
        Rounding::NearestEven,
        Exact::from_parts(x.negative(), magnitude, 0, false),
    )
}

/// An operand or product other than a NaN.
#[derive(Clone, Copy)]
enum Term {
    Infinity(bool),
    Exact(Exact),
}

fn term(operand: Operand) -> Option<Term> {
    match operand {
        Operand::Zero { negative } => Some(Term::Exact(Exact::from_parts(negative, 0, 0, false))),
        Operand::Finite(x) => Some(Term::Exact(x)),
        Operand::Infinity { negative } => Some(Term::Infinity(negative)),
        Operand::NaN { .. } => None,
    }
}

/// The product of two terms, or `None` for infinity times zero.
///
/// [AltiVec-PEM p:3-13 s:3.2.4.2] infinity times zero is invalid.
fn product_term(x: Term, y: Term) -> Option<Term> {
    match (x, y) {
        (Term::Infinity(p), Term::Infinity(q)) => Some(Term::Infinity(p != q)),
        (Term::Infinity(p), Term::Exact(z)) | (Term::Exact(z), Term::Infinity(p)) => {
            (z.significand() != 0).then_some(Term::Infinity(p != z.negative()))
        }
        (Term::Exact(x), Term::Exact(y)) => Some(Term::Exact(product(x, y))),
    }
}

/// The sum of two terms, rounded in `mode`.
///
/// [AltiVec-PEM p:3-13 s:3.2.4.2] a magnitude subtraction of infinities is invalid and gives the default QNaN.
fn sum_terms(mode: VmxMode, x: Term, y: Term) -> u32 {
    match (x, y) {
        (Term::Infinity(p), Term::Infinity(q)) if p != q => VMX_DEFAULT_NAN,
        (Term::Infinity(negative), _) | (_, Term::Infinity(negative)) => {
            0x7F80_0000 | u32::from(negative) << 31
        }
        (Term::Exact(x), Term::Exact(y)) => vmx_round(mode, Rounding::NearestEven, sum(x, y)),
    }
}

/// The sum of two words neither of which is a NaN.
fn add_in(mode: VmxMode, a: u32, b: u32) -> u32 {
    match (term(vmx_unpack(mode, a)), term(vmx_unpack(mode, b))) {
        (Some(x), Some(y)) => sum_terms(mode, x, y),
        _ => VMX_DEFAULT_NAN,
    }
}

/// `a * c + b` on three words none of which is a NaN.
fn madd_in(mode: VmxMode, a: u32, b: u32, c: u32) -> u32 {
    let unpacked = |x: u32| term(vmx_unpack(mode, x));
    match (unpacked(a), unpacked(b), unpacked(c)) {
        (Some(x), Some(y), Some(z)) => {
            product_term(x, z).map_or(VMX_DEFAULT_NAN, |p| sum_terms(mode, p, y))
        }
        _ => VMX_DEFAULT_NAN,
    }
}

/// Rounds `exact` to binary32 in `mode`.
pub fn vmx_round(mode: VmxMode, rounding: Rounding, exact: Exact) -> u32 {
    round_in::<Binary32>(mode, rounding, exact) as u32
}

/// Rounds `exact` into format `F` in `mode`: non-Java mode replaces a
/// value tiny before rounding with a zero of its sign.
///
/// [AltiVec-PEM p:3-14 s:3.2.4.6] underflow is detected before rounding; with `VSCR[NJ]` = 1 the result is a zero with the sign of the intermediate result.
/// The page's own 3.2.1.2 replaces only a result Java mode would deliver denormalized; the ppu_vmx_float_policy console capture settles it as before rounding: a product half a denormal step below 2^-126 gives a zero under NJ = 1.
pub(crate) fn round_in<F: Format>(mode: VmxMode, rounding: Rounding, exact: Exact) -> u64 {
    let tiny = exact.significand() != 0 && binade(exact) < 1 - F::BIAS;
    if mode == VmxMode::NonJava && tiny {
        return u64::from(exact.negative()) << F::SIGN_SHIFT;
    }
    round_pack::<F>(Policy::PowerPc, rounding, exact).bits
}
