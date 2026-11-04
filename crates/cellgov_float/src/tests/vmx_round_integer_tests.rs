//! `vmx_round_integer` against a reference that finds the integer part
//! by binary search over exact comparisons, then the directed cases.

use super::vmx_add_tests::{is_nan, value};
use crate::round::Rounding;
use crate::vmx::{vmx_round_integer, VmxMode};

const MODES: [VmxMode; 2] = [VmxMode::Java, VmxMode::NonJava];
const ROUNDINGS: [Rounding; 4] = [
    Rounding::NearestEven,
    Rounding::TowardZero,
    Rounding::TowardPositive,
    Rounding::TowardNegative,
];

/// The binary32 word of the integer `n` with sign `negative`.
fn integer_word(negative: bool, n: u32) -> u32 {
    let sign = u32::from(negative) << 31;
    if n == 0 {
        return sign;
    }
    let top = 31 - n.leading_zeros();
    // Every n here is below 2^24, so the shift is exact.
    let fraction = (n << (23 - top)) & 0x7F_FFFF;
    sign | (top + 127) << 23 | fraction
}

/// `b` rounded to an integer value from the reference.
///
/// [AltiVec-PEM p:3-15 s:3.2.5.2] an SNaN gives its quiet copy.
/// [AltiVec-PEM p:6-126 s:6.2] vrfin rounds to nearest; ties go to the even integer.
fn reference(mode: VmxMode, rounding: Rounding, b: u32) -> u32 {
    if is_nan(b) {
        return b | 0x40_0000;
    }
    if b & 0x7F80_0000 == 0x7F80_0000 {
        return b;
    }
    let x = value(mode, b);
    if x.significand == 0 {
        return b & 0x8000_0000;
    }
    if x.exponent >= 0 {
        return b;
    }
    let shift = (-x.exponent) as u32;
    // The largest n with n <= |x|: n * 2^shift <= significand.
    let at_most = |n: u128| shift > 100 || n << shift <= x.significand;
    let (mut lo, mut hi) = (0u128, 1u128 << 24);
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        if shift <= 100 && at_most(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let exact = shift <= 100 && lo << shift == x.significand;
    // Twice |x| against the odd 2n + 1, both scaled by 2^shift.
    let above_half = shift <= 100 && 2 * x.significand > (2 * lo + 1) << shift;
    let at_half = shift <= 100 && 2 * x.significand == (2 * lo + 1) << shift;
    let up = !exact
        && match rounding {
            Rounding::NearestEven => above_half || (at_half && lo % 2 == 1),
            Rounding::TowardZero => false,
            Rounding::TowardPositive => !x.negative,
            Rounding::TowardNegative => x.negative,
        };
    integer_word(x.negative, (lo + u128::from(up)) as u32)
}

/// Binary32 words of both signs around every integer boundary.
///
/// - the denormals
/// - values below one half
/// - values near one and near 2^23
/// - the largest finite value
/// - the infinities and two NaNs
fn words() -> Vec<u32> {
    let exponents = [0u32, 1, 100, 125, 126, 127, 128, 129, 149, 150, 151, 254];
    let fractions = [
        0u32, 1, 0x40_0000, 0x40_0001, 0x3F_FFFF, 0x60_0000, 0x7F_FFFF,
    ];
    let mut words: Vec<u32> = exponents
        .iter()
        .flat_map(|&e| fractions.iter().map(move |&f| e << 23 | f))
        .chain([0x7F80_0000, 0x7FC0_0001, 0x7F80_0001])
        .collect();
    words.extend(words.clone().iter().map(|w| w | 0x8000_0000));
    words
}

#[test]
fn every_word_rounds_like_the_reference_in_each_mode_and_rounding() {
    let words = words();
    assert_eq!(words.len(), 2 * (12 * 7 + 3));
    for mode in MODES {
        for rounding in ROUNDINGS {
            for &b in &words {
                assert_eq!(
                    vmx_round_integer(mode, rounding, b),
                    reference(mode, rounding, b),
                    "{mode:?} {rounding:?} {b:#010x}"
                );
            }
        }
    }
}

/// [AltiVec-PEM p:6-126 s:6.2] vrfin rounds n + 0.5 to the even integer.
#[test]
fn halves_round_to_even_under_round_to_nearest() {
    let near = |b| vmx_round_integer(VmxMode::Java, Rounding::NearestEven, b);
    // 0.5, 1.5, 2.5, 3.5 and their negatives.
    let halves = [0x3F00_0000, 0x3FC0_0000, 0x4020_0000, 0x4060_0000];
    let even = [0, 0x4000_0000, 0x4000_0000, 0x4080_0000];
    for (h, e) in halves.into_iter().zip(even) {
        assert_eq!(near(h), e, "{h:#010x}");
        assert_eq!(near(h | 0x8000_0000), e | 0x8000_0000, "-{h:#010x}");
    }
    // Just above and just below one half.
    assert_eq!(near(0x3F00_0001), 0x3F80_0000);
    assert_eq!(near(0x3EFF_FFFF), 0);
}

/// Every magnitude of 2^23 or more is already an integer.
#[test]
fn values_from_two_to_the_23_come_back_unchanged() {
    for mode in MODES {
        for rounding in ROUNDINGS {
            for b in [
                0x4B00_0000,
                0x4B00_0001,
                0x7F7F_FFFF,
                0xCB00_0000,
                0x7F80_0000,
            ] {
                assert_eq!(vmx_round_integer(mode, rounding, b), b, "{rounding:?}");
            }
        }
    }
    // 2^23 - 0.5: a tie between an odd and an even integer.
    let b = 0x4AFF_FFFF;
    assert_eq!(
        vmx_round_integer(VmxMode::Java, Rounding::NearestEven, b),
        0x4B00_0000
    );
    assert_eq!(
        vmx_round_integer(VmxMode::Java, Rounding::TowardZero, b),
        0x4AFF_FFFE
    );
    assert_eq!(
        vmx_round_integer(VmxMode::Java, Rounding::TowardNegative, b),
        0x4AFF_FFFE
    );
    assert_eq!(
        vmx_round_integer(VmxMode::Java, Rounding::TowardPositive, b),
        0x4B00_0000
    );
}

#[test]
fn a_zero_result_keeps_the_sign() {
    let r = |rounding, b| vmx_round_integer(VmxMode::Java, rounding, b);
    // -0.4, -0.7, -0.5 and +0.5.
    assert_eq!(r(Rounding::NearestEven, 0xBECC_CCCD), 0x8000_0000);
    assert_eq!(r(Rounding::TowardZero, 0xBF33_3333), 0x8000_0000);
    assert_eq!(r(Rounding::TowardPositive, 0xBF00_0000), 0x8000_0000);
    assert_eq!(r(Rounding::TowardNegative, 0x3F00_0000), 0);
    // -0.5 toward minus infinity and +0.5 toward plus infinity leave zero.
    assert_eq!(r(Rounding::TowardNegative, 0xBF00_0000), 0xBF80_0000);
    assert_eq!(r(Rounding::TowardPositive, 0x3F00_0000), 0x3F80_0000);
}

/// [AltiVec-PEM p:6-127 s:6.2] vrfip flushes a denormal under NJ = 1;
/// [AltiVec-PEM p:3-11 s:3.2.1] so does every vector instruction, vrfim
/// included.
#[test]
fn a_denormal_rounds_away_from_zero_only_in_java_mode() {
    let (d, nd) = (0x0000_0001, 0x8000_0001);
    let r = |mode, rounding, b| vmx_round_integer(mode, rounding, b);
    assert_eq!(r(VmxMode::Java, Rounding::TowardPositive, d), 0x3F80_0000);
    assert_eq!(r(VmxMode::NonJava, Rounding::TowardPositive, d), 0);
    assert_eq!(r(VmxMode::Java, Rounding::TowardNegative, nd), 0xBF80_0000);
    assert_eq!(
        r(VmxMode::NonJava, Rounding::TowardNegative, nd),
        0x8000_0000
    );
    for mode in MODES {
        for b in [d, nd, 0x007F_FFFF, 0x807F_FFFF] {
            let zero = b & 0x8000_0000;
            assert_eq!(
                r(mode, Rounding::NearestEven, b),
                zero,
                "{mode:?} {b:#010x}"
            );
            assert_eq!(r(mode, Rounding::TowardZero, b), zero, "{mode:?} {b:#010x}");
        }
    }
}

#[test]
fn a_nan_comes_back_quieted() {
    for rounding in ROUNDINGS {
        assert_eq!(
            vmx_round_integer(VmxMode::Java, rounding, 0x7F81_2345),
            0x7FC1_2345
        );
        assert_eq!(
            vmx_round_integer(VmxMode::Java, rounding, 0xFFC0_0042),
            0xFFC0_0042
        );
    }
}
