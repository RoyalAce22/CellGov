//! The vector policies: the tiny format exhaustively, the coverage model
//! at binary32, operand decoding in both modes, the underflow boundary and
//! NaN selection.

use super::oracle::{below_smallest_normal, define, enumerate, Tiny};
use super::round_tests::{coverage, finite, product, sum};
use crate::format::{Binary32, Format};
use crate::operand::Operand;
use crate::round::{Exact, Policy, Rounding};
use crate::vmx::{
    round_in, unpack_in, vmx_flush, vmx_nan, vmx_round, vmx_unpack, VmxMode, VMX_DEFAULT_NAN,
};

const MODES: [VmxMode; 2] = [VmxMode::Java, VmxMode::NonJava];

/// The result `mode` gives `x`, from an oracle: the oracle's rounding in
/// Java mode; a zero of the sign of `x` for a tiny `x` in non-Java mode.
fn expected<F: Format>(mode: VmxMode, x: Exact, oracle: fn(Policy, Rounding, Exact) -> u64) -> u64 {
    if mode == VmxMode::NonJava && below_smallest_normal::<F>(x) {
        u64::from(x.negative) << F::SIGN_SHIFT
    } else {
        oracle(Policy::PowerPc, Rounding::NearestEven, x)
    }
}

fn enumerated<F: Format>(policy: Policy, rounding: Rounding, x: Exact) -> u64 {
    enumerate::<F>(policy, rounding, x).bits
}

fn defined<F: Format>(policy: Policy, rounding: Rounding, x: Exact) -> u64 {
    define::<F>(policy, rounding, x).bits
}

/// The finite operands of `F` that `mode` reads, as exact values.
fn operands<F: Format>(mode: VmxMode) -> Vec<Exact> {
    (0..1u64 << (F::SIGN_SHIFT + 1))
        .filter_map(|bits| match unpack_in::<F>(mode, bits) {
            Operand::Finite(x) => Some(x),
            _ => None,
        })
        .collect()
}

#[test]
fn every_tiny_operand_pair_rounds_like_the_enumeration_oracle_in_both_modes() {
    for mode in MODES {
        let operands = operands::<Tiny>(mode);
        assert_eq!(
            operands.len(),
            if mode == VmxMode::Java {
                2 * 119
            } else {
                2 * 112
            },
            "{mode:?}: finite operands"
        );
        for &a in &operands {
            for &b in &operands {
                for x in [product(a, b), sum(a, b)] {
                    assert_eq!(
                        round_in::<Tiny>(mode, Rounding::NearestEven, x),
                        expected::<Tiny>(mode, x, enumerated::<Tiny>),
                        "{mode:?} {x:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_coverage_model_rounds_like_the_definitional_oracle_in_both_modes() {
    for mode in MODES {
        for x in coverage::<Binary32>(Policy::PowerPc) {
            assert_eq!(
                u64::from(vmx_round(mode, Rounding::NearestEven, x)),
                expected::<Binary32>(mode, x, defined::<Binary32>),
                "{mode:?} {x:?}"
            );
        }
    }
}

/// [AltiVec-PEM p:3-15 s:3.2.5] binary32 holds normalized and denormalized
/// numbers, signed zeros and infinities, QNaNs and SNaNs.
/// [AltiVec-PEM p:3-11 s:3.2.1] non-Java mode flushes a denormalized
/// operand to the correctly signed zero.
#[test]
fn each_operand_class_decodes_in_each_mode() {
    let denormal = |negative, fraction| {
        Operand::Finite(Exact {
            negative,
            significand: fraction,
            exponent: -149,
            sticky: false,
        })
    };
    let normal = |negative, significand, exponent| {
        Operand::Finite(Exact {
            negative,
            significand,
            exponent,
            sticky: false,
        })
    };
    let cases = [
        (
            0x0000_0000,
            Operand::Zero { negative: false },
            Operand::Zero { negative: false },
        ),
        (
            0x8000_0000,
            Operand::Zero { negative: true },
            Operand::Zero { negative: true },
        ),
        (
            0x0000_0001,
            denormal(false, 1),
            Operand::Zero { negative: false },
        ),
        (
            0x807F_FFFF,
            denormal(true, 0x7F_FFFF),
            Operand::Zero { negative: true },
        ),
        (
            0x0080_0000,
            normal(false, 1 << 23, -149),
            normal(false, 1 << 23, -149),
        ),
        (
            0xFF7F_FFFF,
            normal(true, 0xFF_FFFF, 104),
            normal(true, 0xFF_FFFF, 104),
        ),
        (
            0x7F80_0000,
            Operand::Infinity { negative: false },
            Operand::Infinity { negative: false },
        ),
        (
            0xFF80_0000,
            Operand::Infinity { negative: true },
            Operand::Infinity { negative: true },
        ),
        (
            0x7FC0_0001,
            Operand::NaN { quiet: true },
            Operand::NaN { quiet: true },
        ),
        (
            0xFF80_0001,
            Operand::NaN { quiet: false },
            Operand::NaN { quiet: false },
        ),
    ];
    for (bits, java, non_java) in cases {
        assert_eq!(vmx_unpack(VmxMode::Java, bits), java, "{bits:#010x} Java");
        assert_eq!(
            vmx_unpack(VmxMode::NonJava, bits),
            non_java,
            "{bits:#010x} non-Java"
        );
    }
}

/// [AltiVec-PEM p:3-14 s:3.2.4.6] a nonzero intermediate result below
/// 2^-126 underflows before rounding: Java mode denormalizes and rounds
/// it, non-Java mode gives a zero of its sign.
#[test]
fn values_just_below_two_to_the_minus_126_denormalize_or_flush() {
    let value = |negative, significand: u128, exponent| Exact {
        negative,
        significand,
        exponent,
        sticky: false,
    };
    let cases = [
        // (2^24 - 1) * 2^-150: half a denormal step below 2^-126; ties to the even 2^-126.
        (value(false, 0xFF_FFFF, -150), 0x0080_0000),
        (value(true, 0xFF_FFFF, -150), 0x8080_0000),
        // 2^-127 exactly.
        (value(false, 1, -127), 0x0040_0000),
        // (2^25 - 1) * 2^-152: an eighth of a denormal step below 2^-127; rounds to 2^-127.
        (value(false, 0x1FF_FFFF, -152), 0x0040_0000),
        // Below half the smallest denormal: rounds to zero.
        (value(true, 1, -151), 0x8000_0000),
        // 2^-126 itself is normal in both modes.
        (value(false, 1, -126), 0x0080_0000),
    ];
    for (x, java) in cases {
        assert_eq!(
            vmx_round(VmxMode::Java, Rounding::NearestEven, x),
            java,
            "{x:?}"
        );
        let non_java = if x.significand == 1 && x.exponent == -126 {
            java
        } else {
            u32::from(x.negative) << 31
        };
        assert_eq!(
            vmx_round(VmxMode::NonJava, Rounding::NearestEven, x),
            non_java,
            "{x:?}"
        );
    }
}

/// [AltiVec-PEM p:3-15 s:3.2.5.1] NaN precedence vA, vB, vC;
/// [AltiVec-PEM p:3-15 s:3.2.5.2] an SNaN is quieted;
/// [AltiVec-PEM p:3-15 s:3.2.5.3] a QNaN is kept bit for bit.
#[test]
fn the_first_nan_is_the_result_quieted() {
    let (one, inf) = (0x3F80_0000, 0x7F80_0000);
    assert_eq!(vmx_nan(&[one, inf, 0]), None);
    assert_eq!(
        vmx_nan(&[0x7FC1_1111, 0x7F82_2222, 0x7FC3_3333]),
        Some(0x7FC1_1111)
    );
    assert_eq!(vmx_nan(&[one, 0x7F82_2222, 0x7FC3_3333]), Some(0x7FC2_2222));
    assert_eq!(vmx_nan(&[one, inf, 0xFF83_3333]), Some(0xFFC3_3333));
    assert_eq!(vmx_nan(&[0xFFC1_2345]), Some(0xFFC1_2345));
    assert_eq!(vmx_nan(&[0x7F80_0001]), Some(0x7FC0_0001));
    assert_eq!(VMX_DEFAULT_NAN, 0x7FC0_0000);
}

/// [AltiVec-PEM p:3-11 s:3.2.1] NJ = 1 reads a denormal as a zero of its sign.
#[test]
fn a_denormal_flushes_to_a_signed_zero_only_in_non_java_mode() {
    let words = [
        0x0000_0001,
        0x807F_FFFF,
        0x0080_0000,
        0x8000_0000,
        0x7F80_0001,
        0x3F80_0000,
    ];
    let flushed = words.map(|w| vmx_flush(VmxMode::NonJava, w));
    assert_eq!(
        flushed,
        [
            0,
            0x8000_0000,
            0x0080_0000,
            0x8000_0000,
            0x7F80_0001,
            0x3F80_0000
        ]
    );
    assert_eq!(words.map(|w| vmx_flush(VmxMode::Java, w)), words);
}

#[test]
fn the_mode_follows_nj() {
    assert_eq!(VmxMode::from_nj(false), VmxMode::Java);
    assert_eq!(VmxMode::from_nj(true), VmxMode::NonJava);
}

#[test]
fn the_java_mode_operands_match_the_powerpc_policy() {
    for bits in [0x0000_0001u64, 0x8040_0000, 0x3F80_0000] {
        assert_eq!(
            unpack_in::<Binary32>(VmxMode::Java, bits),
            finite::<Binary32>(Policy::PowerPc, bits)
                .map_or(Operand::Zero { negative: false }, Operand::Finite)
        );
    }
}
