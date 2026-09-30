//! `round_pack` against the oracles: exhaustively in a tiny format, over a
//! coverage model at full width, and pinned by a golden hash.

use super::oracle::{define, enumerate, Tiny};
use crate::format::{Binary32, Binary64, Format};
use crate::operand::{default_nan, unpack, Operand};
use crate::round::{round_pack, Exact, Flags, Packed, Policy, Rounding};

const MODES: [Rounding; 4] = [
    Rounding::NearestEven,
    Rounding::TowardZero,
    Rounding::TowardPositive,
    Rounding::TowardNegative,
];

/// The modes that matter under `policy`: single precision always truncates.
fn modes(policy: Policy) -> &'static [Rounding] {
    match policy {
        Policy::SpuExtended => &MODES[1..2],
        Policy::Ieee754Cbe => &MODES,
    }
}

/// The exact product of two finite operands.
fn product(a: Exact, b: Exact) -> Exact {
    Exact {
        negative: a.negative != b.negative,
        significand: a.significand * b.significand,
        exponent: a.exponent + b.exponent,
        sticky: false,
    }
}

/// The exact sum of two finite operands whose exponents lie within 100.
fn sum(a: Exact, b: Exact) -> Exact {
    let e = a.exponent.min(b.exponent);
    let (x, y) = (
        a.significand << (a.exponent - e),
        b.significand << (b.exponent - e),
    );
    let (negative, significand) = if a.negative == b.negative {
        (a.negative, x + y)
    } else if x >= y {
        (a.negative, x - y)
    } else {
        (b.negative, y - x)
    };
    Exact {
        negative,
        significand,
        exponent: e,
        sticky: false,
    }
}

fn finite<F: Format>(policy: Policy, bits: u64) -> Option<Exact> {
    match unpack::<F>(policy, bits).0 {
        Operand::Finite(exact) => Some(exact),
        _ => None,
    }
}

fn check<F: Format>(policy: Policy, x: Exact, oracle: fn(Policy, Rounding, Exact) -> Packed) {
    for &rounding in modes(policy) {
        assert_eq!(
            round_pack::<F>(policy, rounding, x),
            oracle(policy, rounding, x),
            "{policy:?} {rounding:?} {x:?}"
        );
    }
}

#[test]
fn every_tiny_operand_pair_rounds_like_the_enumeration_oracle() {
    for policy in [Policy::SpuExtended, Policy::Ieee754Cbe] {
        let operands: Vec<Exact> = (0..256)
            .filter_map(|bits| finite::<Tiny>(policy, bits))
            .collect();
        for &a in &operands {
            for &b in &operands {
                for x in [product(a, b), sum(a, b)] {
                    check::<Tiny>(policy, x, enumerate::<Tiny>);
                    check::<Tiny>(policy, x, define::<Tiny>);
                }
            }
        }
    }
}

#[test]
fn every_tiny_sticky_value_rounds_like_the_enumeration_oracle() {
    // Six significant bits is the least a sticky value may carry: FRAC_BITS + 3.
    for policy in [Policy::SpuExtended, Policy::Ieee754Cbe] {
        for significand in 32..1024u128 {
            for exponent in -20..=6 {
                for sticky in [false, true] {
                    for negative in [false, true] {
                        let x = Exact {
                            negative,
                            significand,
                            exponent,
                            sticky,
                        };
                        check::<Tiny>(policy, x, enumerate::<Tiny>);
                        check::<Tiny>(policy, x, define::<Tiny>);
                    }
                }
            }
        }
    }
}

/// A deterministic pseudo-random sequence for the coverage model.
fn lcg(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state >> 11
}

/// Exact values that drive every truncation, round and sticky combination,
/// with and without a carry into the exponent, at each exponent boundary,
/// and the products of every pair of operand classes.
fn coverage<F: Format>(policy: Policy) -> Vec<Exact> {
    let frac = F::FRAC_BITS;
    let lead = 1u128 << frac;
    let mask = lead - 1;
    let mut seed = 0x0123_4567_89ab_cdef;
    let mut kept = vec![lead | mask, lead, lead | (0x5555_5555_5555_5555 & mask)];
    for _ in 0..3 {
        kept.push(lead | (u128::from(lcg(&mut seed)) & mask));
    }
    let bias = F::BIAS;
    let top = F::EXP_MAX as i32;
    let targets = [
        -(frac as i32) - 3,
        -1,
        0,
        1,
        2,
        bias,
        top - 2,
        top - 1,
        top,
        top + 1,
    ];
    let mut values = Vec::new();
    for target in targets {
        for &k in &kept {
            for tail in 0..8u128 {
                for sticky in [false, true] {
                    for negative in [false, true] {
                        values.push(Exact {
                            negative,
                            significand: k << 3 | tail,
                            exponent: target - bias - frac as i32 - 3,
                            sticky,
                        });
                    }
                }
            }
        }
    }
    // [Aharoni2003 p:17 s:1] operand classes crossed per operand.
    let fraction = |seed: &mut u64| lcg(seed) & F::FRAC_MASK;
    let exp_max = u64::from(F::EXP_MAX);
    let classes = [
        1,
        fraction(&mut seed) | 1,
        F::FRAC_MASK,
        1 << frac,
        (1 << frac) | 1,
        (u64::from(F::BIAS as u32) << frac) | fraction(&mut seed),
        ((exp_max - 1) << frac) | fraction(&mut seed),
        (exp_max << frac),
        (exp_max << frac) | fraction(&mut seed),
        ((exp_max - 1) << frac) | F::FRAC_MASK,
        (exp_max << frac) | F::FRAC_MASK,
    ];
    let operands: Vec<Exact> = classes
        .iter()
        .flat_map(|&bits| [bits, bits | 1 << F::SIGN_SHIFT])
        .filter_map(|bits| finite::<F>(policy, bits))
        .collect();
    for &a in &operands {
        for &b in &operands {
            values.push(product(a, b));
        }
    }
    values
}

#[test]
fn the_coverage_model_rounds_like_the_definitional_oracle() {
    for x in coverage::<Binary32>(Policy::SpuExtended) {
        check::<Binary32>(Policy::SpuExtended, x, define::<Binary32>);
    }
    for x in coverage::<Binary32>(Policy::Ieee754Cbe) {
        check::<Binary32>(Policy::Ieee754Cbe, x, define::<Binary32>);
    }
    for x in coverage::<Binary64>(Policy::Ieee754Cbe) {
        check::<Binary64>(Policy::Ieee754Cbe, x, define::<Binary64>);
    }
}

fn flag_byte(flags: Flags) -> u8 {
    [
        flags.overflow,
        flags.underflow,
        flags.inexact,
        flags.invalid,
        flags.nan,
        flags.denormal,
        flags.diff,
    ]
    .iter()
    .enumerate()
    .fold(0, |byte, (bit, &set)| byte | u8::from(set) << bit)
}

/// FNV-1a over every coverage result's bits and flags.
fn golden_hash() -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut feed = |bytes: &[u8]| {
        for &byte in bytes {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    let mut record = |packed: Packed| {
        feed(&packed.bits.to_le_bytes());
        feed(&[flag_byte(packed.flags)]);
    };
    for x in coverage::<Binary32>(Policy::SpuExtended) {
        record(round_pack::<Binary32>(
            Policy::SpuExtended,
            Rounding::TowardZero,
            x,
        ));
    }
    for x in coverage::<Binary32>(Policy::Ieee754Cbe) {
        for rounding in MODES {
            record(round_pack::<Binary32>(Policy::Ieee754Cbe, rounding, x));
        }
    }
    for x in coverage::<Binary64>(Policy::Ieee754Cbe) {
        for rounding in MODES {
            record(round_pack::<Binary64>(Policy::Ieee754Cbe, rounding, x));
        }
    }
    hash
}

/// Every host must produce these bits: the pinned hash of the coverage results.
#[test]
fn the_coverage_results_match_their_golden_hash() {
    assert_eq!(golden_hash(), GOLDEN);
}

const GOLDEN: u64 = 0xf18c_7871_88cd_df2f;

// [SPU-ISA p:195 s:9.1] a zero exponent reads as zero and exponent 255 is a normal binade.
#[test]
fn single_precision_operands_read_zero_and_the_extended_binade() {
    let (zero, flags) = unpack::<Binary32>(Policy::SpuExtended, 0x0000_0001);
    assert_eq!(zero, Operand::Zero { negative: false });
    assert!(flags.diff);
    let (top, flags) = unpack::<Binary32>(Policy::SpuExtended, 0x7F80_0000);
    assert_eq!(
        top,
        Operand::Finite(Exact {
            negative: false,
            significand: 1 << 23,
            exponent: 128 - 23,
            sticky: false,
        })
    );
    assert!(flags.diff);
    assert_eq!(
        unpack::<Binary32>(Policy::SpuExtended, 0x3F80_0000).1,
        Flags::default()
    );
}

// [SPU-ISA p:199 s:9.2.2] a denormal operand reads as a zero of its sign and sets DENORM.
// [SPU-ISA p:197 s:9.2] the default QNaN is 0x7FF8000000000000.
#[test]
fn double_precision_operands_read_denormals_as_zero_and_classify_nans() {
    let (zero, flags) = unpack::<Binary64>(Policy::Ieee754Cbe, 0x8000_0000_0000_0001);
    assert_eq!(zero, Operand::Zero { negative: true });
    assert!(flags.denormal);
    assert_eq!(
        unpack::<Binary64>(Policy::Ieee754Cbe, 0xFFF0_0000_0000_0000).0,
        Operand::Infinity { negative: true }
    );
    let (quiet, flags) = unpack::<Binary64>(Policy::Ieee754Cbe, 0x7FF8_0000_0000_0001);
    assert_eq!(quiet, Operand::NaN { quiet: true });
    assert!(flags.nan);
    assert_eq!(
        unpack::<Binary64>(Policy::Ieee754Cbe, 0x7FF0_0000_0000_0001).0,
        Operand::NaN { quiet: false }
    );
    assert_eq!(default_nan::<Binary64>(), 0x7FF8_0000_0000_0000);
    assert_eq!(default_nan::<Binary32>(), 0x7FC0_0000);
}

/// Every single-precision pattern reads back to itself: `unpack` then
/// `round_pack` is the identity on each finite value, under both policies,
/// with DIFF exactly on the extended binade.
// [SPU-ISA p:195 s:9.1] every exponent from 1 to 255 is a normal single-precision binade.
#[test]
#[ignore = "exhaustive over 2^32 patterns; run with --release -- --ignored"]
fn every_single_precision_pattern_round_trips() {
    for bits in 0..=u64::from(u32::MAX) {
        let exponent = bits >> 23 & 0xFF;
        for policy in [Policy::SpuExtended, Policy::Ieee754Cbe] {
            let Operand::Finite(x) = unpack::<Binary32>(policy, bits).0 else {
                continue;
            };
            let packed = round_pack::<Binary32>(policy, Rounding::NearestEven, x);
            assert_eq!(packed.bits, bits, "{policy:?} {bits:#010x}");
            let diff = policy == Policy::SpuExtended && exponent == 0xFF;
            assert_eq!(
                packed.flags,
                Flags {
                    diff,
                    ..Flags::default()
                },
                "{policy:?} {bits:#010x}"
            );
        }
    }
}
