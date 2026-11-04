//! `vmx_madd` and `vmx_nmsub` against the definitional oracle: a cross
//! product of three binary32 operands in both modes, then single
//! rounding, zero signs, NaN precedence and the vnmsubfp negation.

use super::round_tests::{product, sum};
use super::vmx_add_tests::{is_nan, value};
use super::vmx_tests::{defined, expected};
use crate::format::Binary32;
use crate::round::Exact;
use crate::vmx::{vmx_madd, vmx_nmsub, VmxMode};

const MODES: [VmxMode; 2] = [VmxMode::Java, VmxMode::NonJava];

fn binade(x: Exact) -> i32 {
    x.exponent + (128 - x.significand.leading_zeros()) as i32
}

/// `x` and `y`, with the smaller replaced by a stand-in when it lies below 2^floor.
///
/// `floor` is below the larger's lowest bit and below a quarter of a
/// binary32 last place in the binade under the larger. The stand-in is
/// 2^(floor - 2) with the sign of the smaller. The smaller and the
/// stand-in round the sum the same way, and the sum with the stand-in
/// fits a u128.
fn near(x: Exact, y: Exact) -> (Exact, Exact) {
    let (big, small) = if binade(x) >= binade(y) {
        (x, y)
    } else {
        (y, x)
    };
    let floor = big.exponent.min(binade(big) - 27);
    if binade(small) > floor {
        return (big, small);
    }
    (big, Exact::from_parts(small.negative, 1, floor - 2, false))
}

/// `a * c + b` from the oracle.
///
/// [AltiVec-PEM p:3-15 s:3.2.5.1] a NaN result is vA's NaN, else vB's, else vC's, quieted.
/// [AltiVec-PEM p:3-13 s:3.2.4.2] infinity times zero and a magnitude subtraction of infinities give 0x7FC0_0000.
/// [AltiVec-PEM p:6-71 s:6.2] one rounding of the exact value, to nearest.
fn reference(mode: VmxMode, a: u32, b: u32, c: u32) -> u32 {
    for v in [a, b, c] {
        if is_nan(v) {
            return v | 0x40_0000;
        }
    }
    let infinite = |v: u32| v & 0x7FFF_FFFF == 0x7F80_0000;
    let zero = |v: u32| !infinite(v) && value(mode, v).significand == 0;
    if (infinite(a) && zero(c)) || (infinite(c) && zero(a)) {
        return 0x7FC0_0000;
    }
    if infinite(a) || infinite(c) {
        let sign = (a ^ c) & 0x8000_0000;
        if infinite(b) && b & 0x8000_0000 != sign {
            return 0x7FC0_0000;
        }
        return 0x7F80_0000 | sign;
    }
    if infinite(b) {
        return b;
    }
    let (p, q) = (product(value(mode, a), value(mode, c)), value(mode, b));
    if p.significand == 0 && q.significand == 0 {
        return u32::from(p.negative && q.negative) << 31;
    }
    if p.significand == 0 || q.significand == 0 {
        let only = if p.significand == 0 { q } else { p };
        return expected::<Binary32>(mode, only, defined::<Binary32>) as u32;
    }
    let (x, y) = near(p, q);
    let s = sum(x, y);
    if s.significand == 0 {
        return 0;
    }
    expected::<Binary32>(mode, s, defined::<Binary32>) as u32
}

/// `-(a * c - b)` from the oracle: a NaN result keeps its sign.
fn negated_reference(mode: VmxMode, a: u32, b: u32, c: u32) -> u32 {
    let b = if is_nan(b) { b } else { b ^ 0x8000_0000 };
    let r = reference(mode, a, b, c);
    if is_nan(r) {
        r
    } else {
        r ^ 0x8000_0000
    }
}

/// Binary32 words in both signs: a spread of exponent classes with three
/// significands, the infinities, and a QNaN and an SNaN.
fn words() -> Vec<u32> {
    let exponents = [0u32, 1, 24, 100, 126, 127, 150, 254];
    let fractions = [0u32, 1, 0x7F_FFFF];
    let mut words: Vec<u32> = exponents
        .iter()
        .flat_map(|&e| fractions.iter().map(move |&f| e << 23 | f))
        .chain([0x7F80_0000, 0x7FC0_0001, 0x7F80_0001])
        .collect();
    words.extend(words.clone().iter().map(|w| w | 0x8000_0000));
    words
}

#[test]
fn every_operand_triple_matches_the_oracle_in_both_modes() {
    let words = words();
    assert_eq!(words.len(), 2 * (8 * 3 + 3));
    for mode in MODES {
        for &a in &words {
            for &b in &words {
                for &c in &words {
                    assert_eq!(
                        vmx_madd(mode, a, b, c),
                        reference(mode, a, b, c),
                        "{mode:?} {a:#010x} * {c:#010x} + {b:#010x}"
                    );
                    assert_eq!(
                        vmx_nmsub(mode, a, b, c),
                        negated_reference(mode, a, b, c),
                        "{mode:?} -({a:#010x} * {c:#010x} - {b:#010x})"
                    );
                }
            }
        }
    }
}

/// [AltiVec-PEM p:6-71 s:6.2] the product is not rounded before the add.
#[test]
fn the_product_is_rounded_only_with_the_sum() {
    let tiny = 0x2B80_0000; // 2^-40
    for mode in MODES {
        // (1 + 2^-12)^2 = 1 + 2^-11 + 2^-24: a tie alone that would round
        // down to even; + 2^-40 lifts it above the tie.
        assert_eq!(vmx_madd(mode, 0x3F80_0800, tiny, 0x3F80_0800), 0x3F80_1001);
        // (1 + 2^-12)(1 + 2^-11 + 2^-12) = 1 + 2^-10 + 2^-23 + 2^-24: a
        // tie alone that would round up to even; - 2^-40 drops it below.
        assert_eq!(
            vmx_madd(mode, 0x3F80_0800, tiny | 0x8000_0000, 0x3F80_1800),
            0x3F80_2001
        );
        assert_eq!(vmx_madd(mode, 0x3F80_0800, tiny, 0x3F80_1800), 0x3F80_2002);
    }
    // 2^-100 x 2^-40 is below 2^-126 alone, and the sum with 2^-126 is
    // normal: NJ = 1 keeps the product's bits.
    for mode in MODES {
        assert_eq!(
            vmx_madd(mode, 0x0D80_0000, 0x0080_0000, 0x2B80_0000),
            0x0080_0200
        );
    }
}

/// [AltiVec-PEM p:6-71 s:6.2] vB = -0.0 makes vmaddfp a multiply that
/// keeps a -0 product; vB = +0.0 turns it into +0.
#[test]
fn each_zero_product_and_zero_addend_takes_its_sign() {
    let (pz, nz, one) = (0, 0x8000_0000, 0x3F80_0000);
    for mode in MODES {
        let madd = |p: u32, b: u32| vmx_madd(mode, p, b, one);
        assert_eq!(
            [madd(pz, pz), madd(pz, nz), madd(nz, pz), madd(nz, nz)],
            [pz, pz, pz, nz]
        );
        let nmsub = |p: u32, b: u32| vmx_nmsub(mode, p, b, one);
        assert_eq!(
            [nmsub(pz, pz), nmsub(pz, nz), nmsub(nz, pz), nmsub(nz, nz)],
            [nz, nz, pz, nz]
        );
        assert_eq!(vmx_madd(mode, 0xBF80_0000, nz, pz), nz);
    }
}

/// [AltiVec-PEM p:3-15 s:3.2.5.1] vA, then vB, then vC -- the addend
/// before the multiplier.
#[test]
fn the_nan_result_follows_va_vb_vc() {
    let (na, nb, nc, one) = (0x7FC0_00AA, 0xFFC0_00BB, 0x7F80_00CC, 0x3F80_0000);
    for mode in MODES {
        assert_eq!(vmx_madd(mode, na, nb, nc), na);
        assert_eq!(vmx_madd(mode, one, nb, nc), nb);
        assert_eq!(vmx_madd(mode, na, one, nc), na);
        assert_eq!(vmx_madd(mode, one, one, nc), 0x7FC0_00CC);
        assert_eq!(vmx_madd(mode, 0x7F80_00DD, one, one), 0x7FC0_00DD);
        // vnmsubfp leaves a NaN result's sign alone.
        assert_eq!(vmx_nmsub(mode, one, nb, nc), nb);
        assert_eq!(vmx_nmsub(mode, one, one, 0xFF80_00EE), 0xFFC0_00EE);
    }
}

/// [AltiVec-PEM p:6-109 s:6.2] vnmsubfp negates a finite or zero result,
/// and not the default QNaN.
#[test]
fn vnmsubfp_negates_every_result_but_a_nan() {
    let (one, two, three, inf) = (0x3F80_0000, 0x4000_0000, 0x4040_0000, 0x7F80_0000);
    for mode in MODES {
        // -(2 x 3 - 1) = -5.
        assert_eq!(vmx_nmsub(mode, two, one, three), 0xC0A0_0000);
        assert_eq!(vmx_nmsub(mode, one, one, one), 0x8000_0000);
        assert_eq!(vmx_nmsub(mode, inf, one, 0), 0x7FC0_0000);
        // -(inf x 1 - inf) is invalid.
        assert_eq!(vmx_nmsub(mode, inf, inf, one), 0x7FC0_0000);
        // -(inf x 1 - 1) = -inf.
        assert_eq!(vmx_nmsub(mode, inf, one, one), 0xFF80_0000);
    }
}

/// [AltiVec-PEM p:3-13 s:3.2.4.2] infinity times zero and a magnitude
/// subtraction of infinities give 0x7FC0_0000.
#[test]
fn invalid_products_and_sums_give_the_default_nan() {
    let (one, inf, ninf) = (0x3F80_0000, 0x7F80_0000, 0xFF80_0000);
    assert_eq!(vmx_madd(VmxMode::Java, inf, one, 0), 0x7FC0_0000);
    assert_eq!(vmx_madd(VmxMode::Java, 0x8000_0000, one, ninf), 0x7FC0_0000);
    assert_eq!(vmx_madd(VmxMode::Java, inf, ninf, one), 0x7FC0_0000);
    // A denormal is a zero under NJ = 1 only.
    assert_eq!(
        vmx_madd(VmxMode::NonJava, inf, one, 0x0000_0001),
        0x7FC0_0000
    );
    assert_eq!(vmx_madd(VmxMode::Java, inf, one, 0x0000_0001), inf);
}

/// [AltiVec-PEM p:3-14 s:3.2.4.5] an overflow gives a signed infinity.
#[test]
fn an_overflowing_product_gives_a_signed_infinity() {
    let max = 0x7F7F_FFFF;
    for mode in MODES {
        assert_eq!(vmx_madd(mode, max, 0, 0x4000_0000), 0x7F80_0000);
        assert_eq!(vmx_madd(mode, max, 0, 0xC000_0000), 0xFF80_0000);
        assert_eq!(vmx_nmsub(mode, max, 0, 0x4000_0000), 0xFF80_0000);
    }
}

#[test]
#[ignore = "sweeps all 2^32 vA words; run with --release --ignored"]
fn every_va_word_matches_the_oracle() {
    let pairs = [(0x3FC0_0000, 0x8000_0000), (0x3F80_0800, 0x2B80_0000)];
    for mode in MODES {
        for (c, b) in pairs {
            for a in 0..=u32::MAX {
                assert_eq!(
                    vmx_madd(mode, a, b, c),
                    reference(mode, a, b, c),
                    "{mode:?} {a:#010x} * {c:#010x} + {b:#010x}"
                );
            }
        }
    }
}
