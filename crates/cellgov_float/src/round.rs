//! The exact value an operation computes, and the one routine that rounds
//! it into a format.

use crate::format::Format;

/// A finite real value: `(significand + f) * 2^exponent`, negated when
/// `negative`, where `f` is zero, or strictly between zero and one when
/// `sticky` is set.
///
/// `sticky` records nonzero bits below the significand's lowest bit. When
/// it is set, the significand must carry at least `FRAC_BITS + 3`
/// significant bits, so the guard, round and sticky positions all lie
/// below the kept precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exact {
    /// The value's sign; a zero value keeps it for the IEEE policy.
    pub(crate) negative: bool,
    /// The integer significand.
    pub(crate) significand: u128,
    /// The power of two the significand is scaled by.
    pub(crate) exponent: i32,
    /// Nonzero bits lie below the significand's lowest bit.
    pub(crate) sticky: bool,
}

/// The widest significand a value built outside this crate may carry: one
/// bit below the 126-bit precision `add` aligns to, so a sum that drops
/// bits keeps at least 124.
pub(crate) const MAX_SIGNIFICAND_BITS: u32 = 125;

impl Exact {
    /// The exact value `(-1)^negative * significand * 2^exponent`, or `None`
    /// when the significand is wider than 125 bits. Only this crate builds
    /// sticky values, so the invariant above holds for every value.
    pub fn new(negative: bool, significand: u128, exponent: i32) -> Option<Exact> {
        (u128::BITS - significand.leading_zeros() <= MAX_SIGNIFICAND_BITS)
            .then(|| Exact::from_parts(negative, significand, exponent, false))
    }

    /// The same value with the opposite sign.
    pub fn negated(self) -> Exact {
        Exact {
            negative: !self.negative,
            ..self
        }
    }

    /// The value's sign.
    pub fn negative(&self) -> bool {
        self.negative
    }

    /// The integer significand.
    pub fn significand(&self) -> u128 {
        self.significand
    }

    /// The power of two the significand is scaled by.
    pub fn exponent(&self) -> i32 {
        self.exponent
    }

    pub(crate) fn from_parts(
        negative: bool,
        significand: u128,
        exponent: i32,
        sticky: bool,
    ) -> Exact {
        Exact {
            negative,
            significand,
            exponent,
            sticky,
        }
    }

    /// Whether nonzero bits lie below the significand's lowest bit.
    pub fn sticky(&self) -> bool {
        self.sticky
    }
}

/// A rounding direction.
// [SPU-ISA p:197 s:9.2] double precision offers round to nearest even, toward zero, toward +infinity and toward -infinity.
// [SPU-ISA p:200 s:9.3] the FPSCR RN0 and RN1 fields name those four modes per slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rounding {
    /// Round to nearest, ties to even.
    NearestEven,
    /// Round toward zero.
    TowardZero,
    /// Round toward positive infinity.
    TowardPositive,
    /// Round toward negative infinity.
    TowardNegative,
}

/// The rules a format's results follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// SPU extended-range single precision: truncation whatever the
    /// requested mode, zero below Smin, saturation above Smax, exponent
    /// 255 an ordinary binade, and +0 for every zero result.
    // [SPU-ISA p:195 s:9.1] the extended range to Smax and +0 for every zero result.
    // [SPU-ISA p:196 s:9.1] truncation only, denormal results to +0, saturation to Smax, and the OVF, UNF and DIFF flags.
    SpuExtended,
    /// IEEE 754 with the CBE's deviations: rounding per the requested mode,
    /// denormal results, overflow per the mode, and UNF only for a tiny
    /// result that is also inexact.
    // [SPU-ISA p:199 s:9.2.2] UNF is tininess before rounding together with inexactness.
    Ieee754Cbe,
}

/// Exception conditions an operation raises.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Flags {
    /// The result overflowed.
    pub overflow: bool,
    /// The result underflowed.
    pub underflow: bool,
    /// The result is inexact (IEEE policy).
    pub inexact: bool,
    /// The operation is invalid (IEEE policy).
    pub invalid: bool,
    /// An operand was a NaN (IEEE policy).
    pub nan: bool,
    /// A denormal operand was read as zero (IEEE policy).
    pub denormal: bool,
    /// An operand or the result left the IEEE single-precision range
    /// (SPU extended policy).
    pub diff: bool,
}

impl Flags {
    /// The union of two flag sets.
    pub fn or(self, other: Flags) -> Flags {
        Flags {
            overflow: self.overflow || other.overflow,
            underflow: self.underflow || other.underflow,
            inexact: self.inexact || other.inexact,
            invalid: self.invalid || other.invalid,
            nan: self.nan || other.nan,
            denormal: self.denormal || other.denormal,
            diff: self.diff || other.diff,
        }
    }
}

/// A packed result and the flags its rounding raised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Packed {
    /// The encoded result, in the low bits.
    pub bits: u64,
    /// The flags raised.
    pub flags: Flags,
}

/// Rounds `exact` into format `F` under `policy`.
///
/// `rounding` is ignored under [`Policy::SpuExtended`], which always
/// truncates.
pub fn round_pack<F: Format>(policy: Policy, rounding: Rounding, exact: Exact) -> Packed {
    debug_assert!(
        !exact.sticky || significant_bits(exact.significand) >= F::FRAC_BITS + 3,
        "a sticky exact value carries too few significand bits"
    );
    match policy {
        Policy::SpuExtended => round_spu_extended::<F>(exact),
        Policy::Ieee754Cbe => round_ieee::<F>(rounding, exact),
    }
}

/// The number of significant bits in `x`.
fn significant_bits(x: u128) -> u32 {
    u128::BITS - x.leading_zeros()
}

/// The unbiased exponent `e` with `2^e <= |value| < 2^(e+1)`, for a nonzero
/// significand.
fn binade(exact: Exact) -> i32 {
    exact.exponent + significant_bits(exact.significand) as i32 - 1
}

/// `x >> k`, with whether any dropped bit was set.
fn shift_right_sticky(x: u128, k: u32) -> (u128, bool) {
    if k == 0 {
        (x, false)
    } else if k >= u128::BITS {
        (0, x != 0)
    } else {
        (x >> k, x & ((1u128 << k) - 1) != 0)
    }
}

fn sign_bit<F: Format>(negative: bool) -> u64 {
    u64::from(negative) << F::SIGN_SHIFT
}

fn round_spu_extended<F: Format>(exact: Exact) -> Packed {
    if exact.significand == 0 {
        return Packed {
            bits: 0,
            flags: Flags::default(),
        };
    }
    let biased = binade(exact) + F::BIAS;
    let sign = sign_bit::<F>(exact.negative);
    let all_ones = sign | ((1u64 << F::SIGN_SHIFT) - 1);
    if biased > F::EXP_MAX as i32 {
        return Packed {
            bits: all_ones,
            flags: Flags {
                overflow: true,
                diff: true,
                ..Flags::default()
            },
        };
    }
    if biased < 1 {
        return Packed {
            bits: 0,
            flags: Flags {
                underflow: true,
                diff: true,
                ..Flags::default()
            },
        };
    }
    // Truncation keeps the leading FRAC_BITS + 1 bits.
    let drop = significant_bits(exact.significand) as i32 - (F::FRAC_BITS as i32 + 1);
    let (kept, dropped) = if drop >= 0 {
        shift_right_sticky(exact.significand, drop as u32)
    } else {
        (exact.significand << (-drop) as u32, false)
    };
    let bits = sign | (biased as u64) << F::FRAC_BITS | (kept as u64 & F::FRAC_MASK);
    let top_binade = biased == F::EXP_MAX as i32;
    Packed {
        bits,
        flags: Flags {
            // The magnitude before truncation exceeds Smax when it lies in the
            // top binade above the all-ones significand.
            overflow: top_binade && bits == all_ones && (dropped || exact.sticky),
            diff: top_binade,
            ..Flags::default()
        },
    }
}

fn round_ieee<F: Format>(rounding: Rounding, exact: Exact) -> Packed {
    let sign = sign_bit::<F>(exact.negative);
    if exact.significand == 0 {
        return Packed {
            bits: sign,
            flags: Flags::default(),
        };
    }
    let emin = 1 - F::BIAS;
    let binade = binade(exact);
    let tiny = binade < emin;
    // The weight of the result's lowest bit: its binade's, or the denormal
    // step below the normal range.
    let lsb = binade.max(emin) - F::FRAC_BITS as i32;
    let (mut kept, remainder_half, remainder_above_half, inexact) = if exact.exponent >= lsb {
        (
            exact.significand << (exact.exponent - lsb) as u32,
            false,
            false,
            false,
        )
    } else {
        let drop = (lsb - exact.exponent) as u32;
        let (kept, below_guard) = shift_right_sticky(exact.significand, drop);
        let guard = drop <= u128::BITS && (exact.significand >> (drop - 1)) & 1 == 1;
        let lower = shift_right_sticky(exact.significand, drop - 1).1 || exact.sticky;
        let inexact = below_guard || exact.sticky;
        (kept, guard && !lower, guard && lower, inexact)
    };
    let odd = kept & 1 == 1;
    let increment = match rounding {
        Rounding::NearestEven => remainder_above_half || (remainder_half && odd),
        Rounding::TowardZero => false,
        Rounding::TowardPositive => inexact && !exact.negative,
        Rounding::TowardNegative => inexact && exact.negative,
    };
    kept += u128::from(increment);
    // A denormal kept value has no implicit bit; a carry into bit FRAC_BITS
    // lands in the exponent field, which is the next binade.
    let mut biased = if tiny { 0 } else { binade + F::BIAS };
    if !tiny && kept >> (F::FRAC_BITS + 1) != 0 {
        kept >>= 1;
        biased += 1;
    }
    let flags = Flags {
        inexact,
        underflow: tiny && inexact,
        ..Flags::default()
    };
    if biased >= F::EXP_MAX as i32 {
        let toward_zero = match rounding {
            Rounding::NearestEven => false,
            Rounding::TowardZero => true,
            Rounding::TowardPositive => exact.negative,
            Rounding::TowardNegative => !exact.negative,
        };
        let magnitude = if toward_zero {
            ((F::EXP_MAX as u64 - 1) << F::FRAC_BITS) | F::FRAC_MASK
        } else {
            (F::EXP_MAX as u64) << F::FRAC_BITS
        };
        return Packed {
            bits: sign | magnitude,
            flags: Flags {
                overflow: true,
                inexact: true,
                ..flags
            },
        };
    }
    let field = if tiny {
        kept as u64
    } else {
        (biased as u64) << F::FRAC_BITS | (kept as u64 & F::FRAC_MASK)
    };
    Packed {
        bits: sign | field,
        flags,
    }
}
