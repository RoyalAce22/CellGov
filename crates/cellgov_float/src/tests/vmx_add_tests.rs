//! `vmx_add` and `vmx_sub` against the definitional oracle: a cross
//! product of binary32 classes in both modes, then the directed cases.

use super::round_tests::sum;
use super::vmx_tests::{defined, expected};
use crate::format::Binary32;
use crate::round::Exact;
use crate::vmx::{vmx_add, vmx_sub, VmxMode};

const MODES: [VmxMode; 2] = [VmxMode::Java, VmxMode::NonJava];

pub(crate) fn is_nan(x: u32) -> bool {
    x & 0x7F80_0000 == 0x7F80_0000 && x & 0x7F_FFFF != 0
}

/// The exact value of the finite word `x` as `mode` reads it.
pub(crate) fn value(mode: VmxMode, x: u32) -> Exact {
    let negative = x >> 31 == 1;
    let (exponent, fraction) = ((x >> 23 & 0xFF) as i32, u128::from(x & 0x7F_FFFF));
    match exponent {
        0 if mode == VmxMode::NonJava => Exact::from_parts(negative, 0, 0, false),
        0 => Exact::from_parts(negative, fraction, -149, false),
        e => Exact::from_parts(negative, fraction | 1 << 23, e - 150, false),
    }
}

/// `x + y` from the oracle.
///
/// [AltiVec-PEM p:3-15 s:3.2.5.1] a NaN result is vA's NaN, else vB's, quieted.
/// [AltiVec-PEM p:3-13 s:3.2.4.2] an infinity minus an infinity is 0x7FC0_0000.
/// [AltiVec-PEM p:3-12 s:3.2.3] the sum rounds to nearest.
fn reference(mode: VmxMode, x: u32, y: u32) -> u32 {
    for v in [x, y] {
        if is_nan(v) {
            return v | 0x40_0000;
        }
    }
    let infinite = |v: u32| v & 0x7FFF_FFFF == 0x7F80_0000;
    match (infinite(x), infinite(y)) {
        (true, true) if x != y => return 0x7FC0_0000,
        (true, _) => return x,
        (_, true) => return y,
        _ => {}
    }
    let (a, b) = (value(mode, x), value(mode, y));
    if a.significand == 0 && b.significand == 0 {
        // An exact zero sum is -0 only when both zeros are -0.
        return u32::from(a.negative && b.negative) << 31;
    }
    if a.significand == 0 || b.significand == 0 {
        let only = if a.significand == 0 { b } else { a };
        return expected::<Binary32>(mode, only, defined::<Binary32>) as u32;
    }
    // Two binary32 significands more than 100 binades apart: the smaller
    // lies below a quarter of the larger's last place and leaves it as it is.
    if (a.exponent - b.exponent).abs() > 100 {
        return if a.exponent > b.exponent { x } else { y };
    }
    let s = sum(a, b);
    if s.significand == 0 {
        return 0;
    }
    expected::<Binary32>(mode, s, defined::<Binary32>) as u32
}

/// Binary32 words in both signs: every exponent class with several
/// significands, the infinities, and a QNaN and an SNaN.
fn words() -> Vec<u32> {
    let exponents = [0u32, 1, 2, 24, 25, 26, 27, 100, 126, 127, 150, 253, 254];
    let fractions = [0u32, 1, 0x40_0000, 0x55_5555, 0x7F_FFFF, 0x12_3457];
    let mut words: Vec<u32> = exponents
        .iter()
        .flat_map(|&e| fractions.iter().map(move |&f| e << 23 | f))
        .chain([0x7F80_0000, 0x7FC0_0001, 0x7F80_0001])
        .collect();
    words.extend(words.clone().iter().map(|w| w | 0x8000_0000));
    words
}

#[test]
fn every_word_pair_adds_and_subtracts_like_the_oracle_in_both_modes() {
    let words = words();
    assert_eq!(words.len(), 2 * (13 * 6 + 3));
    for mode in MODES {
        for &x in &words {
            for &y in &words {
                assert_eq!(
                    vmx_add(mode, x, y),
                    reference(mode, x, y),
                    "{mode:?} {x:#010x} + {y:#010x}"
                );
                let negated = if is_nan(y) { y } else { y ^ 0x8000_0000 };
                assert_eq!(
                    vmx_sub(mode, x, y),
                    reference(mode, x, negated),
                    "{mode:?} {x:#010x} - {y:#010x}"
                );
            }
        }
    }
}

/// Ties go to even, a value above a tie rounds up, and a carry out of the
/// significand moves the exponent.
#[test]
fn round_to_nearest_takes_guard_round_and_sticky() {
    let one = 0x3F80_0000;
    for mode in MODES {
        // 1 + 2^-24: a tie, kept at the even 1.0.
        assert_eq!(vmx_add(mode, one, 0x3380_0000), one);
        // (1 + 2^-23) + 2^-24: a tie, rounded up to the even 1 + 2^-22.
        assert_eq!(vmx_add(mode, 0x3F80_0001, 0x3380_0000), 0x3F80_0002);
        // 1 + 2^-24 + 2^-47: above the tie.
        assert_eq!(vmx_add(mode, one, 0x3380_0001), 0x3F80_0001);
        // (2 - 2^-23) + 2^-24: carries into the exponent, 2.0.
        assert_eq!(vmx_add(mode, 0x3FFF_FFFF, 0x3380_0000), 0x4000_0000);
        // 1 - 2^-25: below the tie of 1 - 2^-24, back to 1.0.
        assert_eq!(vmx_sub(mode, one, 0x3300_0000), one);
        // 1 - 2^-24 is exact.
        assert_eq!(vmx_sub(mode, one, 0x3380_0000), 0x3F7F_FFFF);
    }
}

/// [AltiVec-PEM p:3-14 s:3.2.4.6] with `VSCR[NJ]` = 1 a result tiny before
/// rounding is a zero of the result's sign.
#[test]
fn a_result_below_two_to_the_minus_126_is_denormal_or_a_signed_zero() {
    let (smallest, one_and_half) = (0x0080_0000, 0x00C0_0000);
    assert_eq!(vmx_sub(VmxMode::Java, one_and_half, smallest), 0x0040_0000);
    assert_eq!(vmx_sub(VmxMode::Java, smallest, one_and_half), 0x8040_0000);
    assert_eq!(vmx_sub(VmxMode::NonJava, one_and_half, smallest), 0);
    assert_eq!(
        vmx_sub(VmxMode::NonJava, smallest, one_and_half),
        0x8000_0000
    );
}

/// [AltiVec-PEM p:3-11 s:3.2.1] non-Java mode reads a denormal operand as
/// a zero of its sign.
#[test]
fn a_denormal_operand_counts_only_in_java_mode() {
    let (d, one) = (0x0000_0001, 0x3F80_0000);
    assert_eq!(vmx_add(VmxMode::Java, d, d), 0x0000_0002);
    assert_eq!(vmx_add(VmxMode::NonJava, d, d), 0);
    assert_eq!(
        vmx_add(VmxMode::NonJava, d | 0x8000_0000, d | 0x8000_0000),
        0x8000_0000
    );
    assert_eq!(vmx_add(VmxMode::Java, 0x0080_0000, d), 0x0080_0001);
    assert_eq!(vmx_add(VmxMode::NonJava, 0x0080_0000, d), 0x0080_0000);
    assert_eq!(vmx_add(VmxMode::NonJava, one, d), one);
}

/// [AltiVec-PEM p:3-14 s:3.2.4.5] an overflow gives an infinity of the
/// result's sign.
#[test]
fn an_overflow_gives_a_signed_infinity() {
    let max = 0x7F7F_FFFF;
    for mode in MODES {
        assert_eq!(vmx_add(mode, max, max), 0x7F80_0000);
        assert_eq!(vmx_sub(mode, max | 0x8000_0000, max), 0xFF80_0000);
        // Half the last place of the largest finite value: a tie, and the
        // odd significand rounds up into the infinity.
        assert_eq!(vmx_add(mode, max, 0x7300_0000), 0x7F80_0000);
        assert_eq!(vmx_add(mode, max, 0x72FF_FFFF), max);
    }
}

/// [AltiVec-PEM p:3-13 s:3.2.4.2] a magnitude subtraction of infinities
/// gives 0x7FC0_0000.
#[test]
fn infinities_follow_ieee() {
    let (inf, ninf) = (0x7F80_0000, 0xFF80_0000);
    for mode in MODES {
        assert_eq!(vmx_add(mode, inf, ninf), 0x7FC0_0000);
        assert_eq!(vmx_add(mode, ninf, inf), 0x7FC0_0000);
        assert_eq!(vmx_sub(mode, inf, inf), 0x7FC0_0000);
        assert_eq!(vmx_add(mode, inf, inf), inf);
        assert_eq!(vmx_sub(mode, inf, ninf), inf);
        assert_eq!(vmx_sub(mode, 0x3F80_0000, inf), ninf);
        assert_eq!(vmx_add(mode, ninf, 0x7F7F_FFFF), ninf);
    }
}

/// [AltiVec-PEM p:3-15 s:3.2.5.1] vA's NaN wins over vB's;
/// [AltiVec-PEM p:3-15 s:3.2.5.2] an SNaN gives its quiet copy. vsubfp passes a NaN
/// in vB through with its own sign.
#[test]
fn the_first_nan_is_the_result_quieted() {
    let one = 0x3F80_0000;
    for mode in MODES {
        assert_eq!(vmx_add(mode, 0x7FC1_1111, one), 0x7FC1_1111);
        assert_eq!(vmx_add(mode, one, 0xFF82_2222), 0xFFC2_2222);
        assert_eq!(vmx_add(mode, 0x7F83_3333, 0x7FC4_4444), 0x7FC3_3333);
        assert_eq!(vmx_sub(mode, one, 0xFF85_5555), 0xFFC5_5555);
        assert_eq!(vmx_sub(mode, one, 0x7FC6_6666), 0x7FC6_6666);
        assert_eq!(vmx_sub(mode, 0x7F80_0000, 0x7F87_7777), 0x7FC7_7777);
    }
}

/// Round to nearest gives +0 to an exact zero sum unless both terms are -0.
#[test]
fn each_zero_sum_takes_its_ieee_sign() {
    let (pz, nz, one) = (0, 0x8000_0000, 0x3F80_0000);
    for mode in MODES {
        assert_eq!(vmx_sub(mode, one, one), pz);
        assert_eq!(vmx_add(mode, one | nz, one), pz);
        assert_eq!(vmx_add(mode, nz, nz), nz);
        assert_eq!(vmx_sub(mode, nz, pz), nz);
        assert_eq!(vmx_sub(mode, nz, nz), pz);
        assert_eq!(vmx_add(mode, pz, nz), pz);
        assert_eq!(vmx_sub(mode, pz, pz), pz);
    }
}
