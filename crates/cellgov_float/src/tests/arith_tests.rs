//! `add` and `mul`: rounding their results agrees with rounding the exact
//! value, and every sticky sum keeps the bits the rounding needs.

use super::oracle::{enumerate, Tiny};
use crate::arith;
use crate::format::{Binary64, Format};
use crate::operand::{unpack, Operand};
use crate::round::{round_pack, Exact, Policy, Rounding};

/// A value the tests know is narrow enough.
fn new(negative: bool, significand: u128, exponent: i32) -> Exact {
    Exact::new(negative, significand, exponent).unwrap()
}

/// A sum of operands the tests know `add` takes.
fn add(a: Exact, b: Exact) -> Exact {
    arith::add(a, b).unwrap()
}

/// A product of operands the tests know `mul` takes.
fn mul(a: Exact, b: Exact) -> Exact {
    arith::mul(a, b).unwrap()
}

const MODES: [Rounding; 4] = [
    Rounding::NearestEven,
    Rounding::TowardZero,
    Rounding::TowardPositive,
    Rounding::TowardNegative,
];

fn finite<F: Format>(policy: Policy, bits: u64) -> Option<Exact> {
    match unpack::<F>(policy, bits).0 {
        Operand::Finite(exact) => Some(exact),
        _ => None,
    }
}

/// The exact sum of two values whose exponents lie within 100.
fn exact_sum(a: Exact, b: Exact) -> Exact {
    let e = a.exponent.min(b.exponent);
    let (x, y) = (
        a.significand << (a.exponent - e),
        b.significand << (b.exponent - e),
    );
    if a.negative == b.negative {
        new(a.negative, x + y, e)
    } else if x >= y {
        new(a.negative, x - y, e)
    } else {
        new(b.negative, y - x, e)
    }
}

#[test]
fn every_tiny_sum_and_product_rounds_like_its_exact_value() {
    for policy in [Policy::SpuExtended, Policy::Ieee754Cbe] {
        let operands: Vec<Exact> = (0..256)
            .filter_map(|bits| finite::<Tiny>(policy, bits))
            .collect();
        for &a in &operands {
            for &b in &operands {
                for rounding in MODES {
                    let exact = exact_sum(a, b);
                    if exact.significand != 0 {
                        assert_eq!(
                            round_pack::<Tiny>(policy, rounding, add(a, b)),
                            enumerate::<Tiny>(policy, rounding, exact),
                            "{policy:?} {rounding:?} {a:?} + {b:?}"
                        );
                    }
                    let product = mul(a, b);
                    assert_eq!(
                        round_pack::<Tiny>(policy, rounding, product),
                        enumerate::<Tiny>(policy, rounding, product),
                        "{policy:?} {rounding:?} {a:?} * {b:?}"
                    );
                }
            }
        }
    }
}

/// A far addend moves the sum strictly off the larger operand, toward the
/// addend's side, by less than the finest step any format sees.
#[test]
fn a_far_addend_rounds_like_a_value_just_off_the_larger_operand() {
    for policy in [Policy::SpuExtended, Policy::Ieee754Cbe] {
        for gap in [120, 126, 127, 128, 140, 200, 1000] {
            for big in 1..16u128 {
                for small in [1u128, 7, 15] {
                    for (big_negative, small_negative) in
                        [(false, false), (false, true), (true, false), (true, true)]
                    {
                        let a = new(big_negative, big, 0);
                        let b = new(small_negative, small, -gap);
                        // big +/- (something in (0, 2^-8)): strictly inside the
                        // unit at 2^-8 above or below big.
                        let scaled = big << 8;
                        let interval = if big_negative == small_negative {
                            Exact::from_parts(big_negative, scaled, -8, true)
                        } else {
                            Exact::from_parts(big_negative, scaled - 1, -8, true)
                        };
                        for rounding in MODES {
                            assert_eq!(
                                round_pack::<Tiny>(policy, rounding, add(a, b)),
                                enumerate::<Tiny>(policy, rounding, interval),
                                "{policy:?} {rounding:?} gap {gap} {a:?} + {b:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn every_sticky_sum_keeps_124_significant_bits() {
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        state
    };
    let mut sticky = 0;
    for _ in 0..20_000 {
        let (Some(a), Some(b)) = (
            finite::<Binary64>(Policy::Ieee754Cbe, next()),
            finite::<Binary64>(Policy::Ieee754Cbe, next()),
        ) else {
            continue;
        };
        for x in [add(a, b), add(mul(a, a), b)] {
            if x.sticky() {
                sticky += 1;
                assert!(128 - x.significand.leading_zeros() >= 124, "{x:?}");
            }
        }
    }
    assert!(sticky > 1000, "only {sticky} sums were sticky");
}

/// The worst cancellation a widest operand allows: 1 - (2^125 - 1) * 2^-126
/// is 1/2 + 2^-126, and alignment drops the addend's lowest bit.
#[test]
fn a_widest_operand_that_loses_bits_cancels_one_leading_bit() {
    let one = new(false, 1, 0);
    let widest = new(true, (1u128 << 125) - 1, -126);
    let sum = add(one, widest);
    assert_eq!(sum, Exact::from_parts(false, 1u128 << 124, -125, true));
    assert_eq!(128 - sum.significand.leading_zeros(), 125);
}

#[test]
fn an_exactly_cancelling_sum_is_positive_zero() {
    let x = new(false, 5, 3);
    let minus_x = new(true, 5, 3);
    let zero = add(x, minus_x);
    assert_eq!((zero.negative, zero.significand), (false, 0));
    let both_negative_zero = add(new(true, 0, 0), new(true, 0, 0));
    assert!(both_negative_zero.negative);
}

/// Nothing outside the crate can hand `round_pack` a value that breaks the
/// sticky invariant: a wide significand, a wide product and a sticky operand
/// are all refused.
#[test]
fn wide_and_sticky_operands_are_refused() {
    assert!(Exact::new(false, 1 << 125, 0).is_none());
    assert!(Exact::new(false, (1 << 125) - 1, 0).is_some());
    let wide = new(false, (1 << 70) - 1, 0);
    assert!(arith::mul(wide, wide).is_none());
    // A sticky sum 125 bits wide, so only its sticky bit refuses it.
    let sticky = add(new(false, 1, 0), new(true, (1 << 125) - 1, -126));
    assert!(sticky.sticky());
    assert_eq!(128 - sticky.significand.leading_zeros(), 125);
    assert!(arith::add(sticky, new(false, 1, 0)).is_none());
    assert!(arith::mul(sticky, new(false, 1, 0)).is_none());
}
