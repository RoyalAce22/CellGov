//! `ppc_div`, `ppc_sqrt` and `ppc_div_sqrt_precision` against the
//! definitional oracle and the documents.
//!
//! The tests build each exact quotient and root by bit-by-bit long
//! division and a digit-by-digit square root, independent of the
//! widened integer division and `isqrt` that the engine uses.

use super::oracle::{define, rounds_up};
use super::round_tests::finite;
use crate::arith::square_root;
use crate::format::{Binary32, Binary64, Format};
use crate::operand::{unpack, Operand};
use crate::ppc::{PpcInvalid, PpcPrecision, PpcResult, PpcStatus, PPC_GENERATED_NAN};
use crate::ppc_arith::{ppc_div, ppc_div_sqrt_precision, ppc_sqrt};
use crate::round::{round_pack, Exact, Policy, Rounding};

const MODES: [Rounding; 4] = [
    Rounding::NearestEven,
    Rounding::TowardZero,
    Rounding::TowardPositive,
    Rounding::TowardNegative,
];

const NEG: u64 = 1 << 63;
const ONE: u64 = 0x3FF0_0000_0000_0000;
const TWO: u64 = 0x4000_0000_0000_0000;
const INF: u64 = 0x7FF0_0000_0000_0000;
const QNAN: u64 = 0x7FF8_0000_0000_0AAA;
const SNAN: u64 = 0x7FF0_0000_0000_0BBB;

/// Positive finite nonzero operands inside the single exponents: values
/// with and without bits below single precision, and values whose
/// quotients and roots fall near a representable number or halfway.
const SINGLE_RANGE: [u64; 10] = [
    ONE,
    TWO,
    0x4008_0000_0000_0000, // 3
    0x3FF0_0000_0000_0001, // 1 + 2^-52
    0x3FF0_0000_0000_0003, // 1 + 3 x 2^-52
    0x3FFF_FFFF_FFFF_FFFF, // just below 2
    0x3FF8_0000_0000_0000, // 1.5
    0x37D0_0000_0000_0000, // 2^-130
    0x47EF_FFFF_E000_0000, // the largest single
    0x4024_0000_0000_0000, // 10
];

/// Positive finite operands only the double forms take.
const DOUBLE_ONLY: [u64; 4] = [
    0x0000_0000_0000_0001, // the smallest denormal
    0x000F_FFFF_FFFF_FFFF, // the largest denormal
    0x0010_0000_0000_0000, // the smallest normal
    0x7FEF_FFFF_FFFF_FFFF, // the largest double
];

/// The exact value of a positive finite nonzero double-format operand.
fn exact(bits: u64) -> Exact {
    finite::<Binary64>(Policy::PowerPc, bits).unwrap()
}

/// `a / b` by bit-by-bit long division to 70 quotient bits, sticky when a
/// remainder is left.
fn long_divide(a: Exact, b: Exact) -> Exact {
    let (mut remainder, mut divisor) = (a.significand(), b.significand());
    // Scale until divisor <= remainder < 2 x divisor, so the first
    // quotient bit is 1 and has weight 2^exponent.
    let mut exponent = a.exponent() - b.exponent();
    while remainder < divisor {
        remainder <<= 1;
        exponent -= 1;
    }
    while remainder >= divisor << 1 {
        divisor <<= 1;
        exponent += 1;
    }
    let mut q: u128 = 0;
    for _ in 0..70 {
        q <<= 1;
        if remainder >= divisor {
            remainder -= divisor;
            q |= 1;
        }
        remainder <<= 1;
    }
    let x = Exact::new(a.negative() != b.negative(), q, exponent - 69).unwrap();
    if remainder != 0 {
        sticky_of(x)
    } else {
        x
    }
}

/// `x` with its sticky bit set.
fn sticky_of(x: Exact) -> Exact {
    Exact::from_parts(x.negative(), x.significand(), x.exponent(), true)
}

/// The square root of a positive `a` by the digit-by-digit method to 62
/// or 63 root bits, sticky when a remainder is left.
fn digit_root(a: Exact) -> Exact {
    // Scale to an even exponent with the significand at 124 or 125 bits,
    // then take 63 root digits from the top two bits at a time.
    let bits = 128 - a.significand().leading_zeros();
    let mut shift = 124 - bits;
    if (a.exponent() - shift as i32) % 2 != 0 {
        shift += 1;
    }
    let n = a.significand() << shift;
    let (mut root, mut remainder) = (0u128, 0u128);
    for i in (0..63).rev() {
        remainder = (remainder << 2) | ((n >> (2 * i)) & 3);
        let trial = (root << 2) | 1;
        root <<= 1;
        if remainder >= trial {
            remainder -= trial;
            root |= 1;
        }
    }
    let x = Exact::new(false, root, (a.exponent() - shift as i32) / 2).unwrap();
    if remainder != 0 {
        sticky_of(x)
    } else {
        x
    }
}

/// The binary32 pattern `bits` in double format.
fn widen(bits: u64) -> u64 {
    let sign = (bits >> 31 & 1) << 63;
    match unpack::<Binary32>(Policy::PowerPc, bits).0 {
        Operand::Finite(x) => {
            round_pack::<Binary64>(Policy::PowerPc, Rounding::NearestEven, x).bits
        }
        Operand::Infinity { .. } => sign | INF,
        _ => sign,
    }
}

/// The oracle's result for the exact value `x` in `precision`.
fn oracle(precision: PpcPrecision, rounding: Rounding, x: Exact) -> PpcResult {
    fn round<F: Format>(rounding: Rounding, x: Exact) -> (u64, PpcStatus) {
        let want = define::<F>(Policy::PowerPc, rounding, x);
        let status = PpcStatus {
            fr: want.flags.overflow || rounds_up::<F>(x, want.bits),
            fi: want.flags.inexact,
            ox: want.flags.overflow,
            ux: want.flags.underflow,
            ..PpcStatus::default()
        };
        (want.bits, status)
    }
    let (bits, status) = match precision {
        PpcPrecision::Double => round::<Binary64>(rounding, x),
        PpcPrecision::Single => {
            let (bits, status) = round::<Binary32>(rounding, x);
            (widen(bits), status)
        }
    };
    PpcResult { bits, status }
}

/// [PPC-Book1 p:112 s:4.6.5.1] fdiv rounds the exact quotient once to the target precision by FPSCR[RN].
/// [PPC-Book1 p:125 s:5.2.1.1] fsqrt rounds the exact root once the same way.
/// [Verdonk2001Basic p:111 s:7.2] quotients and roots near a representable number and near halfway.
/// [Aharoni2003 p:17 s:1] operand classes crossed per operand.
#[test]
fn finite_quotients_and_roots_round_like_the_oracle_in_every_mode_and_width() {
    let single: Vec<u64> = SINGLE_RANGE.iter().flat_map(|&v| [v, v | NEG]).collect();
    let double: Vec<u64> = single
        .iter()
        .copied()
        .chain(DOUBLE_ONLY.iter().flat_map(|&v| [v, v | NEG]))
        .collect();
    let mut checked = 0;
    for (precision, operands) in [
        (PpcPrecision::Single, &single),
        (PpcPrecision::Double, &double),
    ] {
        for &a in operands.iter() {
            for rounding in MODES {
                if a & NEG == 0 {
                    let want = oracle(precision, rounding, digit_root(exact(a)));
                    assert_eq!(
                        ppc_sqrt(precision, rounding, a),
                        want,
                        "sqrt {precision:?} {rounding:?} {a:#018x}"
                    );
                    checked += 1;
                }
                for &b in operands.iter() {
                    let want = oracle(precision, rounding, long_divide(exact(a), exact(b)));
                    assert_eq!(
                        ppc_div(precision, rounding, a, b),
                        want,
                        "div {precision:?} {rounding:?} {a:#018x} {b:#018x}"
                    );
                    checked += 1;
                }
            }
        }
    }
    // Single: 20 x 20 quotients and 10 roots per mode; double: 28 x 28 and 14.
    assert_eq!(checked, 4 * (20 * 20 + 10) + 4 * (28 * 28 + 14));
}

/// [Verdonk2001Basic p:111 s:7.2] the square root of a t-bit number is never exactly halfway between two t-bit numbers.
/// An exact root of a 53-bit significand has at most 27 significant
/// bits, below any double halfway point.
#[test]
fn a_root_is_exact_and_short_or_sticky() {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut checked = 0;
    for k in 0..20_000u64 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        // Every fourth operand is a perfect square, so both arms run.
        let bits = if k % 4 == 0 {
            let r = u128::from(1 + (state >> 38));
            let scale = (state & 0xFF) as i32 - 128;
            let square = Exact::new(false, r * r, 2 * scale).unwrap();
            round_pack::<Binary64>(Policy::PowerPc, Rounding::NearestEven, square).bits
        } else {
            (state >> 2) & 0x7FEF_FFFF_FFFF_FFFF | 1
        };
        let x = square_root(exact(bits));
        assert_eq!(
            x.sticky(),
            digit_root(exact(bits)).sticky(),
            "{bits:#018x}: exactness"
        );
        if !x.sticky() {
            let s = x.significand() >> x.significand().trailing_zeros();
            assert!(128 - s.leading_zeros() <= 27, "{bits:#018x}: {x:?}");
        }
        checked += 1;
    }
    assert_eq!(checked, 20_000);
}

/// [PPC-Book1 p:97 s:4.4.1] zero over zero, infinity over infinity, an SNaN and the square root of a negative nonzero number are invalid.
/// [PPC-Book1 p:98 s:4.4.2] a finite nonzero dividend over a zero is a zero divide, giving an infinity signed by the operands.
/// [PPC-Book1 p:92 s:4.3.3] the square root of -0 is -0.
#[test]
fn the_special_operands_give_the_architected_results_and_causes() {
    let cause = |f: fn(&mut PpcInvalid)| {
        let mut c = PpcInvalid::default();
        f(&mut c);
        PpcStatus {
            invalid: c,
            ..PpcStatus::default()
        }
    };
    let none = PpcStatus::default();
    let zx = PpcStatus {
        zx: true,
        ..PpcStatus::default()
    };
    let result = |bits, status| PpcResult { bits, status };
    for precision in [PpcPrecision::Double, PpcPrecision::Single] {
        let div = |a, b| ppc_div(precision, Rounding::NearestEven, a, b);
        let sqrt = |b| ppc_sqrt(precision, Rounding::NearestEven, b);
        let zdz = cause(|c| c.zero_div_zero = true);
        let idi = cause(|c| c.inf_div_inf = true);
        let vxsqrt = cause(|c| c.sqrt = true);
        let snan = cause(|c| c.snan = true);
        assert_eq!(div(0, NEG), result(PPC_GENERATED_NAN, zdz));
        assert_eq!(div(NEG | INF, INF), result(PPC_GENERATED_NAN, idi));
        assert_eq!(div(ONE, 0), result(INF, zx));
        assert_eq!(div(NEG | ONE, 0), result(NEG | INF, zx));
        assert_eq!(div(ONE, NEG), result(NEG | INF, zx));
        assert_eq!(div(INF, 0), result(INF, none));
        assert_eq!(div(INF, NEG | TWO), result(NEG | INF, none));
        assert_eq!(div(NEG | ONE, INF), result(NEG, none));
        assert_eq!(div(NEG, TWO), result(NEG, none));
        assert_eq!(div(SNAN, 0), result(SNAN | 1 << 51, snan));
        assert_eq!(div(QNAN, SNAN), result(QNAN, snan));
        assert_eq!(div(ONE, QNAN | NEG), result(QNAN | NEG, none));
        assert_eq!(sqrt(NEG), result(NEG, none));
        assert_eq!(sqrt(0), result(0, none));
        assert_eq!(sqrt(INF), result(INF, none));
        assert_eq!(sqrt(NEG | INF), result(PPC_GENERATED_NAN, vxsqrt));
        assert_eq!(sqrt(NEG | 1), result(PPC_GENERATED_NAN, vxsqrt));
        assert_eq!(sqrt(SNAN), result(SNAN | 1 << 51, snan));
        assert_eq!(sqrt(QNAN | NEG), result(QNAN | NEG, none));
    }
}

/// The root of the double (2^24 + 1)^2 is exact and lies halfway between
/// two singles, so fsqrts rounds it as a tie: to even in nearest mode.
///
/// [PPC-Book1 p:125 s:5.2.1.1] fsqrts rounds the root to single precision by FPSCR[RN].
#[test]
fn a_single_root_of_a_double_square_can_be_an_exact_tie() {
    let root = (1u128 << 24) + 1;
    let square = Exact::new(false, root * root, 0).unwrap();
    let bits = round_pack::<Binary64>(Policy::PowerPc, Rounding::NearestEven, square).bits;
    assert_eq!(bits, 0x42F0_0000_2000_0010);
    assert!(!square_root(exact(bits)).sticky());
    let low = 0x4170_0000_0000_0000; // 2^24
    let high = 0x4170_0000_2000_0000; // 2^24 + 2
    let inexact = |fr| PpcStatus {
        fr,
        fi: true,
        ..PpcStatus::default()
    };
    for (rounding, want, fr) in [
        (Rounding::NearestEven, low, false),
        (Rounding::TowardZero, low, false),
        (Rounding::TowardPositive, high, true),
        (Rounding::TowardNegative, low, false),
    ] {
        let got = ppc_sqrt(PpcPrecision::Single, rounding, bits);
        assert_eq!(
            got,
            PpcResult {
                bits: want,
                status: inexact(fr)
            },
            "{rounding:?}"
        );
        assert_eq!(
            got,
            oracle(PpcPrecision::Single, rounding, digit_root(exact(bits)))
        );
    }
    assert_eq!(
        ppc_sqrt(PpcPrecision::Double, Rounding::NearestEven, bits),
        PpcResult {
            bits: 0x4170_0000_1000_0000, // 2^24 + 1
            status: PpcStatus::default(),
        }
    );
}

/// The ppu_fp_divsqrt_modes console capture: fdivs and fsqrts round to
/// double precision when an operand lies outside the single exponents,
/// and to single precision otherwise.
#[test]
fn a_single_divide_or_root_of_an_out_of_range_operand_rounds_to_double() {
    let single = PpcPrecision::Single;
    let double = PpcPrecision::Double;
    for (operands, want) in [
        (&[0x47F0_0000_0000_0000, ONE][..], double), // 2^128
        (&[ONE, 0x3690_0000_0000_0000][..], double), // 2^-150
        (&[0x0010_0000_0000_0000][..], double),
        (&[1][..], double),
        (&[0x47E0_0000_0000_0000, ONE][..], single), // 2^127
        (&[0x36A0_0000_0000_0000][..], single),      // 2^-149
        (&[0, INF, QNAN, NEG | ONE][..], single),
    ] {
        assert_eq!(
            ppc_div_sqrt_precision(single, operands),
            want,
            "{operands:#x?}"
        );
        assert_eq!(ppc_div_sqrt_precision(double, operands), double);
    }
}

#[test]
#[ignore = "takes 2^33 square roots; run with --release --ignored"]
fn every_single_operand_root_rounds_like_the_oracle() {
    let mut checked = 0u64;
    for word in 1..0x7F80_0000u64 {
        let bits = widen(word);
        let x = digit_root(exact(bits));
        for rounding in MODES {
            assert_eq!(
                ppc_sqrt(PpcPrecision::Single, rounding, bits),
                oracle(PpcPrecision::Single, rounding, x),
                "{rounding:?} {word:#010x}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 4 * (0x7F80_0000 - 1));
}
