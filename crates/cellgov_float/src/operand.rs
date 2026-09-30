//! Operand decoding: what an encoded value means under a policy.

use crate::format::Format;
use crate::round::{Exact, Flags, Policy};

/// A decoded operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operand {
    /// A zero, with its sign.
    Zero {
        /// The encoded sign.
        negative: bool,
    },
    /// A nonzero finite value.
    Finite(Exact),
    /// An infinity (IEEE policy only).
    Infinity {
        /// The encoded sign.
        negative: bool,
    },
    /// A NaN (IEEE policy only).
    NaN {
        /// The fraction's leading bit is set.
        quiet: bool,
    },
}

/// Decodes `bits` in format `F` under `policy`, with the flags reading it
/// raises.
///
/// Under [`Policy::SpuExtended`] an exponent-0 operand is zero whatever its
/// fraction, and one with a nonzero fraction or with exponent 255 raises
/// DIFF. Under [`Policy::Ieee754Cbe`] a denormal operand is a zero of its
/// sign and raises DENORM, and a NaN raises NaN.
// [SPU-ISA p:195 s:9.1] a zero exponent is zero and exponent 255 is a normal binade in single precision.
// [SPU-ISA p:196 s:9.1] a denormal single operand reads as zero; DIFF marks an input with a maximal exponent or a zero exponent and nonzero fraction.
// [SPU-ISA p:199 s:9.2.2] a denormal double operand is read as zero and sets DENORM; a NaN operand sets NaN.
pub fn unpack<F: Format>(policy: Policy, bits: u64) -> (Operand, Flags) {
    let negative = bits >> F::SIGN_SHIFT & 1 == 1;
    let exponent = (bits >> F::FRAC_BITS) as u32 & F::EXP_MAX;
    let fraction = bits & F::FRAC_MASK;
    let normal = |biased: u32| {
        Operand::Finite(Exact {
            negative,
            significand: u128::from(fraction | 1 << F::FRAC_BITS),
            exponent: biased as i32 - F::BIAS - F::FRAC_BITS as i32,
            sticky: false,
        })
    };
    match (policy, exponent) {
        (Policy::SpuExtended, 0) => (
            Operand::Zero { negative },
            Flags {
                diff: fraction != 0,
                ..Flags::default()
            },
        ),
        (Policy::SpuExtended, e) => (
            normal(e),
            Flags {
                diff: e == F::EXP_MAX,
                ..Flags::default()
            },
        ),
        (Policy::Ieee754Cbe, 0) => (
            Operand::Zero { negative },
            Flags {
                denormal: fraction != 0,
                ..Flags::default()
            },
        ),
        (Policy::Ieee754Cbe, e) if e == F::EXP_MAX && fraction == 0 => {
            (Operand::Infinity { negative }, Flags::default())
        }
        (Policy::Ieee754Cbe, e) if e == F::EXP_MAX => (
            Operand::NaN {
                quiet: fraction >> (F::FRAC_BITS - 1) & 1 == 1,
            },
            Flags {
                nan: true,
                ..Flags::default()
            },
        ),
        (Policy::Ieee754Cbe, e) => (normal(e), Flags::default()),
    }
}

/// Decodes `bits` in format `F` under [`Policy::SpuExtended`], which reads
/// every pattern as a zero or a finite value, with the flags reading it
/// raises.
// [SPU-ISA p:195 s:9.1] single precision has no infinity or NaN: exponent 255 is a normal binade.
pub fn unpack_extended<F: Format>(bits: u64) -> (Exact, Flags) {
    let (operand, flags) = unpack::<F>(Policy::SpuExtended, bits);
    let exact = match operand {
        Operand::Finite(exact) => exact,
        // `unpack` returns no infinity or NaN under this policy.
        Operand::Zero { negative } | Operand::Infinity { negative } => {
            Exact::from_parts(negative, 0, 0, false)
        }
        Operand::NaN { .. } => Exact::from_parts(false, 0, 0, false),
    };
    (exact, flags)
}

/// The default quiet NaN every NaN result takes: a positive sign, an
/// all-ones exponent and only the fraction's leading bit set.
// [SPU-ISA p:197 s:9.2] the default QNaN has a zero sign, an all-ones exponent and only the fraction's leading bit set (0x7FF8000000000000 for double precision); an implementation may return it for any NaN result.
// [Mueller2005 p:61 s:3.2] the CBE double-precision unit returns the generic NaN for every NaN result.
pub fn default_nan<F: Format>() -> u64 {
    (F::EXP_MAX as u64) << F::FRAC_BITS | 1 << (F::FRAC_BITS - 1)
}
