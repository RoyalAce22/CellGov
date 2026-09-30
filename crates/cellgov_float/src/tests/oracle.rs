//! Reference rounding for the tests: an enumeration oracle that picks the
//! result from every representable value of a small format, and a
//! definitional oracle that floors at the result's unit in the last place.

use crate::format::Format;
use crate::round::{Exact, Flags, Packed, Policy, Rounding};
use std::cmp::Ordering;

/// An 8-bit format small enough to enumerate: 4 exponent bits, 3 fraction bits.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Tiny;

impl Format for Tiny {
    const EXP_BITS: u32 = 4;
    const FRAC_BITS: u32 = 3;
}

/// A value `significand * 2^exponent`.
#[derive(Debug, Clone, Copy)]
struct Value {
    significand: u128,
    exponent: i32,
}

/// Compares two values exactly; both exponents must lie within 100 of each other.
fn compare(a: Value, b: Value) -> Ordering {
    let e = a.exponent.min(b.exponent);
    (a.significand << (a.exponent - e)).cmp(&(b.significand << (b.exponent - e)))
}

/// Compares `|x|` with `c`. A sticky `x` lies strictly between its
/// significand and the next multiple of its lowest bit; `c` is a multiple of
/// that bit or smaller, so equality with the significand means `x` is above.
fn compare_exact(x: Exact, c: Value) -> Ordering {
    let a = Value {
        significand: x.significand,
        exponent: x.exponent,
    };
    match compare(a, c) {
        Ordering::Equal if x.sticky => Ordering::Greater,
        other => other,
    }
}

/// The magnitude an encoded pattern of `F` holds, when finite.
fn magnitude<F: Format>(bits: u64) -> Value {
    let exponent = (bits >> F::FRAC_BITS) as i32;
    let fraction = u128::from(bits & F::FRAC_MASK);
    if exponent == 0 {
        Value {
            significand: fraction,
            exponent: 1 - F::BIAS - F::FRAC_BITS as i32,
        }
    } else {
        Value {
            significand: fraction | 1 << F::FRAC_BITS,
            exponent: exponent - F::BIAS - F::FRAC_BITS as i32,
        }
    }
}

/// Picks the result for `x` from the ascending magnitude patterns `patterns`
/// of format `F`, rounding in `rounding`. The final entry of `patterns` is
/// the overflow point: a value no finite result reaches.
fn pick<F: Format>(x: Exact, rounding: Rounding, patterns: &[u64]) -> (u64, bool, bool) {
    let values: Vec<Value> = patterns.iter().map(|&bits| magnitude::<F>(bits)).collect();
    // The last pattern below or at |x|, and whether |x| is exact there.
    let mut lower = 0;
    for (index, value) in values.iter().enumerate() {
        if compare_exact(x, *value) != Ordering::Less {
            lower = index;
        }
    }
    let hit = compare_exact(x, values[lower]) == Ordering::Equal;
    if hit {
        return (patterns[lower], false, lower == patterns.len() - 1);
    }
    let upper = (lower + 1).min(patterns.len() - 1);
    let up = match rounding {
        Rounding::TowardZero => false,
        Rounding::TowardPositive => !x.negative,
        Rounding::TowardNegative => x.negative,
        Rounding::NearestEven => {
            let (lo, hi) = (values[lower], values[upper]);
            // (lo + hi) / 2, exactly.
            let e = lo.exponent.min(hi.exponent);
            let mid = Value {
                significand: (lo.significand << (lo.exponent - e))
                    + (hi.significand << (hi.exponent - e)),
                exponent: e - 1,
            };
            match compare_exact(x, mid) {
                Ordering::Less => false,
                Ordering::Greater => true,
                Ordering::Equal => patterns[lower] & 1 == 1,
            }
        }
    };
    let chosen = if up { upper } else { lower };
    let overflow = chosen == patterns.len() - 1 || lower == patterns.len() - 1;
    (patterns[chosen], true, overflow)
}

/// Rounds `x` by enumerating every result pattern of `F`. `F` must be small.
pub(crate) fn enumerate<F: Format>(policy: Policy, rounding: Rounding, x: Exact) -> Packed {
    let sign = u64::from(x.negative) << F::SIGN_SHIFT;
    let exp_max = u64::from(F::EXP_MAX);
    let count = 1u64 << F::SIGN_SHIFT;
    if x.significand == 0 {
        let bits = if policy == Policy::SpuExtended {
            0
        } else {
            sign
        };
        return Packed {
            bits,
            flags: Flags::default(),
        };
    }
    match policy {
        Policy::SpuExtended => {
            let smin = 1u64 << F::FRAC_BITS;
            let smax = count - 1;
            if compare_exact(x, magnitude::<F>(smin)) == Ordering::Less {
                return Packed {
                    bits: 0,
                    flags: Flags {
                        underflow: true,
                        diff: true,
                        ..Flags::default()
                    },
                };
            }
            let over = compare_exact(x, magnitude::<F>(smax)) == Ordering::Greater;
            // Truncation: the largest normal at or below |x|.
            let normals: Vec<u64> = (smin..count).collect();
            let mut chosen = smin;
            for &bits in &normals {
                if compare_exact(x, magnitude::<F>(bits)) != Ordering::Less {
                    chosen = bits;
                }
            }
            Packed {
                bits: sign | chosen,
                flags: Flags {
                    overflow: over,
                    diff: chosen >> F::FRAC_BITS == exp_max,
                    ..Flags::default()
                },
            }
        }
        Policy::Ieee754Cbe => {
            // Every finite magnitude, then the infinity pattern as the
            // overflow point: 2^(emax + 1), the next binade after the largest.
            let patterns: Vec<u64> = (0..=exp_max << F::FRAC_BITS).collect();
            let (chosen, inexact, overflow) = pick::<F>(x, rounding, &patterns);
            let smallest_normal = magnitude::<F>(1 << F::FRAC_BITS);
            let tiny = compare_exact(x, smallest_normal) == Ordering::Less;
            let bits = if overflow {
                let toward_zero = match rounding {
                    Rounding::NearestEven => false,
                    Rounding::TowardZero => true,
                    Rounding::TowardPositive => x.negative,
                    Rounding::TowardNegative => !x.negative,
                };
                if toward_zero {
                    (exp_max << F::FRAC_BITS) - 1
                } else {
                    exp_max << F::FRAC_BITS
                }
            } else {
                chosen
            };
            Packed {
                bits: sign | bits,
                flags: Flags {
                    overflow,
                    inexact: inexact || overflow,
                    underflow: tiny && inexact,
                    ..Flags::default()
                },
            }
        }
    }
}

/// Rounds `x` by its definition: floor at the unit in the last place of the
/// result's binade, then step up per the mode. Works at any width.
pub(crate) fn define<F: Format>(policy: Policy, rounding: Rounding, x: Exact) -> Packed {
    let sign = u64::from(x.negative) << F::SIGN_SHIFT;
    if x.significand == 0 {
        let bits = if policy == Policy::SpuExtended {
            0
        } else {
            sign
        };
        return Packed {
            bits,
            flags: Flags::default(),
        };
    }
    let width = 128 - x.significand.leading_zeros() as i32;
    let binade = x.exponent + width - 1;
    let frac = F::FRAC_BITS as i32;
    let emin = 1 - F::BIAS;
    let rounding = if policy == Policy::SpuExtended {
        Rounding::TowardZero
    } else {
        rounding
    };
    if policy == Policy::SpuExtended {
        if binade < emin {
            return Packed {
                bits: 0,
                flags: Flags {
                    underflow: true,
                    diff: true,
                    ..Flags::default()
                },
            };
        }
        if binade > F::EXP_MAX as i32 - F::BIAS {
            return Packed {
                bits: sign | ((1 << F::SIGN_SHIFT) - 1),
                flags: Flags {
                    overflow: true,
                    diff: true,
                    ..Flags::default()
                },
            };
        }
    }
    // The unit in the last place: 2^q.
    let q = binade.max(emin) - frac;
    // floor(|x| / 2^q) and the remainder's position against half a unit.
    let (floor, rest) = if x.exponent >= q {
        (x.significand << (x.exponent - q), Ordering::Less)
    } else {
        let shift = (q - x.exponent) as u32;
        let floor = if shift >= 128 {
            0
        } else {
            x.significand >> shift
        };
        let remainder = if shift >= 128 {
            x.significand
        } else {
            x.significand & ((1u128 << shift) - 1)
        };
        let rest = if remainder == 0 && !x.sticky {
            Ordering::Less
        } else if shift > 128 {
            // Every set bit lies below 2^127 <= half a unit.
            Ordering::Less
        } else {
            // Compare the remainder with half a unit, 2^(shift - 1).
            let half = 1u128 << (shift - 1);
            match remainder.cmp(&half) {
                Ordering::Equal if x.sticky => Ordering::Greater,
                other => other,
            }
        };
        (floor, rest)
    };
    let exact = x.exponent >= q || {
        let shift = (q - x.exponent) as u32;
        let remainder = if shift >= 128 {
            x.significand
        } else {
            x.significand & ((1u128 << shift) - 1)
        };
        remainder == 0 && !x.sticky
    };
    // `rest` is Less for below half (or exact), Equal at half, Greater above.
    let up = !exact
        && match rounding {
            Rounding::TowardZero => false,
            Rounding::TowardPositive => !x.negative,
            Rounding::TowardNegative => x.negative,
            Rounding::NearestEven => match rest {
                Ordering::Greater => true,
                Ordering::Equal => floor & 1 == 1,
                Ordering::Less => false,
            },
        };
    let units = floor + u128::from(up);
    // Re-encode units * 2^q.
    let tiny = binade < emin;
    let mut biased = if tiny { 0 } else { binade + F::BIAS };
    let mut units = units;
    if !tiny && units >> (F::FRAC_BITS + 1) != 0 {
        units >>= 1;
        biased += 1;
    }
    let (field, overflow) = match policy {
        Policy::SpuExtended => {
            let top = F::EXP_MAX as i32;
            let bits = (biased as u64) << F::FRAC_BITS | (units as u64 & F::FRAC_MASK);
            let smax = (1 << F::SIGN_SHIFT) - 1;
            return Packed {
                bits: sign | bits,
                flags: Flags {
                    overflow: biased == top && bits == smax && !exact,
                    diff: biased == top,
                    ..Flags::default()
                },
            };
        }
        Policy::Ieee754Cbe => {
            if biased >= F::EXP_MAX as i32 {
                let toward_zero = match rounding {
                    Rounding::NearestEven => false,
                    Rounding::TowardZero => true,
                    Rounding::TowardPositive => x.negative,
                    Rounding::TowardNegative => !x.negative,
                };
                let exp_max = u64::from(F::EXP_MAX);
                let bits = if toward_zero {
                    (exp_max << F::FRAC_BITS) - 1
                } else {
                    exp_max << F::FRAC_BITS
                };
                (bits, true)
            } else if tiny {
                (units as u64, false)
            } else {
                (
                    (biased as u64) << F::FRAC_BITS | (units as u64 & F::FRAC_MASK),
                    false,
                )
            }
        }
    };
    Packed {
        bits: sign | field,
        flags: Flags {
            overflow,
            inexact: !exact || overflow,
            underflow: tiny && !exact,
            ..Flags::default()
        },
    }
}
