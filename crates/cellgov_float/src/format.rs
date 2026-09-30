//! Binary interchange formats, described by their field widths.

/// A binary floating-point format: a sign bit, an exponent field and a
/// fraction field, packed into the low bits of a `u64`.
///
/// The widths are parameters so a test can instantiate a format small
/// enough to check exhaustively.
pub trait Format {
    /// Exponent field width in bits.
    const EXP_BITS: u32;
    /// Fraction field width in bits, without the implicit leading one.
    const FRAC_BITS: u32;
    /// The exponent bias.
    const BIAS: i32 = (1 << (Self::EXP_BITS - 1)) - 1;
    /// The all-ones exponent field.
    const EXP_MAX: u32 = (1 << Self::EXP_BITS) - 1;
    /// The sign bit's position.
    const SIGN_SHIFT: u32 = Self::EXP_BITS + Self::FRAC_BITS;
    /// Mask of the fraction field.
    const FRAC_MASK: u64 = (1 << Self::FRAC_BITS) - 1;
}

/// The 32-bit format, single precision on the SPU.
///
/// [SPU-ISA p:195 s:9.1] single precision: 1 sign bit, 8 exponent bits biased by 127, 23 fraction bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binary32;

impl Format for Binary32 {
    const EXP_BITS: u32 = 8;
    const FRAC_BITS: u32 = 23;
}

/// The 64-bit format, double precision on the SPU.
///
/// [SPU-ISA p:197 s:9.2] double precision follows the IEEE 754 double format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binary64;

impl Format for Binary64 {
    const EXP_BITS: u32 = 11;
    const FRAC_BITS: u32 = 52;
}
