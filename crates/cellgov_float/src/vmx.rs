//! The vector single-precision rules: binary32 under [`Policy::PowerPc`],
//! with the non-Java mode that `VSCR[NJ]` selects.
//!
//! The vector unit keeps no floating-point status, so these functions
//! return bits only.

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
