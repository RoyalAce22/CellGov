//! Exact arithmetic on unrounded values: the products and sums an
//! operation hands to `round_pack`.
//!
//! `add` is the one producer of a sticky value. It shifts the larger
//! operand to 126 significant bits before it aligns the smaller one, so
//! bits are shifted out only when the operands are at least 126 bits apart.
//! Each operand has at most 125 significant bits, so a subtraction then
//! cancels at most one leading bit, and every sticky sum keeps at least 124
//! significant bits, beyond any format's guard, round and sticky positions.

use crate::round::{Exact, MAX_SIGNIFICAND_BITS};

/// The precision `add` normalizes the larger operand to.
const ALIGN_BITS: u32 = 126;

/// The widest operand `add` takes and the widest product `mul` forms. One
/// bit below `ALIGN_BITS`: a 126-bit operand that loses bits in alignment
/// can cancel every bit of the larger operand.
const OPERAND_BITS: u32 = MAX_SIGNIFICAND_BITS;

/// Whether `x` is an operand `add` and `mul` take: exact and narrow enough.
fn operand(x: Exact) -> bool {
    !x.sticky && significant_bits(x.significand) <= OPERAND_BITS
}

fn significant_bits(x: u128) -> u32 {
    u128::BITS - x.leading_zeros()
}

/// `x >> k`, with whether any dropped bit was set.
fn shift_right_sticky(x: u128, k: u64) -> (u128, bool) {
    if k == 0 {
        (x, false)
    } else if k >= u64::from(u128::BITS) {
        (0, x != 0)
    } else {
        (x >> k, x & ((1u128 << k) - 1) != 0)
    }
}

/// The exact product of two exact values, or `None` when an operand is
/// sticky or the product would be wider than 125 bits. The operands
/// `unpack` decodes always multiply: at most 106 bits for two doubles.
pub fn mul(a: Exact, b: Exact) -> Option<Exact> {
    if !operand(a) || !operand(b) {
        return None;
    }
    if significant_bits(a.significand) + significant_bits(b.significand) > OPERAND_BITS {
        return None;
    }
    Exact::new(
        a.negative != b.negative,
        a.significand * b.significand,
        a.exponent + b.exponent,
    )
}

/// The sum of two exact values: exact, or sticky when the smaller operand
/// lies partly below the larger's 126-bit precision.
///
/// Returns `None` when an operand is sticky or wider than 125 bits; every
/// operand `unpack` decodes and every product `mul` forms is neither. An exact zero sum
/// is +0 unless both operands are negative; a caller that rounds toward
/// negative infinity supplies its own -0 for `x - x`.
///
/// [SPU-ISA p:199 s:9.2.2] underflow is tininess together with an inexact result, so the bits below the kept precision must survive as a sticky bit.
pub fn add(a: Exact, b: Exact) -> Option<Exact> {
    if !operand(a) || !operand(b) {
        return None;
    }
    Some(sum(a, b))
}

/// The sum of two operands `add` has checked.
fn sum(a: Exact, b: Exact) -> Exact {
    if a.significand == 0 && b.significand == 0 {
        return Exact::from_parts(
            a.negative && b.negative,
            0,
            a.exponent.min(b.exponent),
            false,
        );
    }
    if b.significand == 0 {
        return a;
    }
    if a.significand == 0 {
        return b;
    }
    let binade = |x: Exact| i64::from(x.exponent) + i64::from(significant_bits(x.significand));
    let (big, small) = if binade(a) >= binade(b) {
        (a, b)
    } else {
        (b, a)
    };
    // The larger operand at 126 significant bits, by an exact left shift.
    let shift = ALIGN_BITS.saturating_sub(significant_bits(big.significand));
    let big_sig = big.significand << shift;
    let big_exp = i64::from(big.exponent) - i64::from(shift);
    // The smaller operand at the same scale. Its binade is at most the
    // larger's, so a left shift keeps it below bit 126; a right shift keeps
    // what it drops as sticky.
    let gap = big_exp - i64::from(small.exponent);
    let (small_sig, sticky) = if gap <= 0 {
        (small.significand << (-gap) as u32, false)
    } else {
        shift_right_sticky(small.significand, gap as u64)
    };
    let exponent = big_exp as i32;
    if big.negative == small.negative {
        return Exact::from_parts(big.negative, big_sig + small_sig, exponent, sticky);
    }
    // Opposite signs: big - (small_sig + e) with 0 < e < 1 is
    // (big_sig - small_sig - 1) + (1 - e). Bits are dropped only when the
    // operands are 126 bits apart, so then big_sig exceeds small_sig.
    match big_sig.cmp(&small_sig) {
        std::cmp::Ordering::Greater => Exact::from_parts(
            big.negative,
            big_sig - small_sig - u128::from(sticky),
            exponent,
            sticky,
        ),
        std::cmp::Ordering::Equal => Exact::from_parts(false, 0, exponent, false),
        std::cmp::Ordering::Less => {
            Exact::from_parts(small.negative, small_sig - big_sig, exponent, false)
        }
    }
}
