//! The PowerPC rules against the definitional oracle and the documents:
//! FR and FI, overflow and the enables, single precision in double
//! format, NaN selection, the exact zero sum and the invalid causes.

use super::oracle::{define, rounds_up, Tiny};
use super::round_tests::{coverage, finite, product, sum};
use crate::format::{Binary32, Binary64, Format};
use crate::ppc::{
    ppc_frsp_nan, ppc_invalid_product, ppc_invalid_quotient, ppc_invalid_sqrt, ppc_invalid_sum,
    ppc_nan_result, ppc_round, ppc_round_in, ppc_sum, PpcEnabled, PpcEnables, PpcInvalid,
    PpcPrecision, PpcStatus, PPC_GENERATED_NAN,
};
use crate::round::{round_pack, Exact, Policy, Rounding};

const MODES: [Rounding; 4] = [
    Rounding::NearestEven,
    Rounding::TowardZero,
    Rounding::TowardPositive,
    Rounding::TowardNegative,
];

const NONE: PpcEnables = PpcEnables {
    invalid: false,
    overflow: false,
    underflow: false,
    zero_divide: false,
    inexact: false,
};

/// Checks `ppc_round_in::<F>` on `x` in every mode against the
/// definitional oracle: the bits, FR from the chosen magnitude, FI, OX
/// and UX.
fn check<F: Format>(x: Exact) {
    for rounding in MODES {
        let got = ppc_round_in::<F>(rounding, x, NONE).unwrap();
        let want = define::<F>(Policy::PowerPc, rounding, x);
        assert_eq!(got.bits, want.bits, "{rounding:?} {x:?}");
        let fr = want.flags.overflow || rounds_up::<F>(x, want.bits);
        assert_eq!(
            got.status,
            PpcStatus {
                fr,
                fi: want.flags.inexact,
                ox: want.flags.overflow,
                ux: want.flags.underflow,
                ..PpcStatus::default()
            },
            "{rounding:?} {x:?}"
        );
    }
}

/// [PPC-Book1 p:94 s:4.3.6] FR is set when rounding increments the fraction and FI when the result is inexact.
/// The tiny format is small enough to check every operand pair.
/// [Verdonk2001Basic p:94 s:1] the rounding rules are format-independent, so they can be tested at any precision and exponent range.
#[test]
fn fr_and_fi_follow_the_oracle_on_every_tiny_value() {
    let operands: Vec<Exact> = (0..256)
        .filter_map(|bits| finite::<Tiny>(Policy::PowerPc, bits))
        .collect();
    let mut checked = 0;
    for &a in &operands {
        for &b in &operands {
            for x in [product(a, b), sum(a, b)] {
                check::<Tiny>(x);
                checked += 1;
            }
        }
    }
    for significand in 32..1024u128 {
        for exponent in -20..=6 {
            for sticky in [false, true] {
                check::<Tiny>(Exact::from_parts(false, significand, exponent, sticky));
                check::<Tiny>(Exact::from_parts(true, significand, exponent, sticky));
                checked += 2;
            }
        }
    }
    assert_eq!(checked, 2 * operands.len().pow(2) + 4 * 992 * 27);
}

/// [Verdonk2001Basic p:103 s:4.1] every round and sticky combination, with and without a carry into the exponent.
/// [Aharoni2003 p:17 s:1] operand classes crossed per operand.
#[test]
fn fr_and_fi_follow_the_oracle_over_the_coverage_model() {
    let single = coverage::<Binary32>(Policy::PowerPc);
    let double = coverage::<Binary64>(Policy::PowerPc);
    assert!(single.len() > 1000 && double.len() > 1000);
    for x in single {
        check::<Binary32>(x);
    }
    for x in double {
        check::<Binary64>(x);
    }
}

/// [PPC-Book1 p:98 s:4.4.3] a disabled overflow gives infinity in round to nearest, the largest finite number toward zero, and by sign toward either infinity; OX and FI are set.
/// The ppu_reference_vectors console capture reads FR = 1 after each of these overflows, in all four modes and both widths.
#[test]
fn a_disabled_overflow_rounds_by_mode_and_sets_fr_and_fi() {
    const DOUBLE_INF: u64 = 0x7FF0_0000_0000_0000;
    const DOUBLE_MAX: u64 = 0x7FEF_FFFF_FFFF_FFFF;
    const SINGLE_MAX: u64 = 0x47EF_FFFF_E000_0000;
    for (precision, exponent, max) in [
        (PpcPrecision::Double, 2000, DOUBLE_MAX),
        (PpcPrecision::Single, 200, SINGLE_MAX),
    ] {
        for negative in [false, true] {
            let x = Exact::new(negative, 1, exponent).unwrap();
            for rounding in MODES {
                let toward_zero = match rounding {
                    Rounding::NearestEven => false,
                    Rounding::TowardZero => true,
                    Rounding::TowardPositive => negative,
                    Rounding::TowardNegative => !negative,
                };
                let magnitude = if toward_zero { max } else { DOUBLE_INF };
                let got = ppc_round(precision, rounding, x, NONE).unwrap();
                assert_eq!(
                    got.bits,
                    u64::from(negative) << 63 | magnitude,
                    "{precision:?} {rounding:?} {negative}"
                );
                assert_eq!(
                    got.status,
                    PpcStatus {
                        fr: true,
                        fi: true,
                        ox: true,
                        ..PpcStatus::default()
                    },
                    "{precision:?} {rounding:?} {negative}"
                );
            }
        }
    }
}

/// [PPC-Book1 p:98 s:4.4.3] OE = 1 scales the exponent of an overflowed result.
/// [PPC-Book1 p:98 s:4.4.4] with UE = 1 an underflow is any tiny result, exact or not.
/// [PPC-Book1 p:99 s:4.4.4] UE = 1 scales the exponent of an underflowed result.
/// [PPC-Book1 p:99 s:4.4.5] XE changes no result.
#[test]
fn an_enabled_overflow_or_underflow_refuses_and_xe_changes_nothing() {
    let huge = Exact::new(false, 1, 2000).unwrap();
    let tiny_exact = Exact::new(false, 1, -1050).unwrap();
    let third = Exact::new(false, 0x5555_5555_5555_5555_5555, -80).unwrap();
    let overflow = PpcEnables {
        overflow: true,
        ..NONE
    };
    let underflow = PpcEnables {
        underflow: true,
        ..NONE
    };
    let inexact = PpcEnables {
        inexact: true,
        ..NONE
    };
    let round = |x, enables| ppc_round(PpcPrecision::Double, Rounding::NearestEven, x, enables);
    assert_eq!(round(huge, overflow), Err(PpcEnabled::Overflow));
    assert_eq!(round(tiny_exact, underflow), Err(PpcEnabled::Underflow));
    assert!(round(huge, underflow).unwrap().status.ox);
    let quiet = round(tiny_exact, NONE).unwrap();
    assert_eq!(quiet.bits, 1 << (1074 - 1050));
    assert_eq!(quiet.status, PpcStatus::default());
    assert_eq!(round(third, inexact), round(third, NONE));
    assert!(round(third, NONE).unwrap().status.fi);
}

/// [PPC-Book1 p:93 s:4.3.5.1] a single-precision result is stored in double format with the low-order 29 fraction bits zero.
#[test]
fn a_single_result_is_the_binary32_rounding_in_double_format() {
    let mut checked = 0;
    for x in coverage::<Binary32>(Policy::PowerPc) {
        for rounding in MODES {
            let got = ppc_round(PpcPrecision::Single, rounding, x, NONE).unwrap();
            assert_eq!(got.bits & ((1 << 29) - 1), 0, "{rounding:?} {x:?}");
            let single = round_pack::<Binary32>(Policy::PowerPc, rounding, x).bits;
            let (sign, rest) = (single >> 31, single & 0x7FFF_FFFF);
            assert_eq!(got.bits >> 63, sign, "{rounding:?} {x:?}");
            if (0x0080_0000..0x7F80_0000).contains(&rest) {
                let exponent = (rest >> 23) as i64 - 127 + 1023;
                let widened = (exponent as u64) << 52 | (rest & 0x7F_FFFF) << 29;
                assert_eq!(got.bits & !(1 << 63), widened, "{rounding:?} {x:?}");
            }
            checked += 1;
        }
    }
    assert!(checked > 4000);
    // A single denormal is a normal double: 2^-149 and 3 x 2^-149.
    let denormal = |s| {
        ppc_round(
            PpcPrecision::Single,
            Rounding::NearestEven,
            Exact::new(false, s, -149).unwrap(),
            NONE,
        )
        .unwrap()
        .bits
    };
    assert_eq!(denormal(1), 0x36A0_0000_0000_0000);
    assert_eq!(denormal(3), 0x36B8_0000_0000_0000);
}

/// [PPC-Book1 p:91 s:4.3.2] a NaN in FRA is the result, else one in FRB, else one in FRC, with the quiet bit set.
#[test]
fn the_first_nan_of_fra_frb_frc_is_the_result_quieted() {
    let nans = [
        0x7FF0_0000_0000_0001u64,
        0x7FF8_0000_0000_0002,
        0xFFF0_0000_0000_0003,
    ];
    let one = 0x3FF0_0000_0000_0000u64;
    for subset in 1..8u32 {
        let pick = |i: usize| {
            if subset >> i & 1 == 1 {
                nans[i]
            } else {
                one
            }
        };
        let first = (0..3).find(|&i| subset >> i & 1 == 1).unwrap();
        assert_eq!(
            ppc_nan_result(Some(pick(0)), Some(pick(1)), Some(pick(2))),
            Some(nans[first] | 1 << 51),
            "subset {subset:#05b}"
        );
    }
    assert_eq!(ppc_nan_result(Some(one), Some(one), Some(one)), None);
    assert_eq!(
        ppc_nan_result(None, Some(nans[2]), None),
        Some(0xFFF8_0000_0000_0003)
    );
    assert_eq!(PPC_GENERATED_NAN, 0x7FF8_0000_0000_0000);
}

/// [PPC-Book1 p:91 s:4.3.2] frsp stores (FRB) bits 0:34 and 29 zero bits.
#[test]
fn frsp_of_a_nan_quiets_it_and_clears_the_low_29_bits() {
    assert_eq!(ppc_frsp_nan(0x7FF0_0001_2345_6789), 0x7FF8_0001_2000_0000);
    assert_eq!(ppc_frsp_nan(0xFFF8_0000_1FFF_FFFF), 0xFFF8_0000_0000_0000);
}

/// [PPC-Book1 p:92 s:4.3.3] an exact zero sum of opposite signs is + in every mode except toward -infinity.
#[test]
fn an_exact_zero_sum_is_negative_only_toward_minus_infinity() {
    let one = Exact::new(false, 1, 0).unwrap();
    let zero = |negative| Exact::new(negative, 0, 0).unwrap();
    for rounding in MODES {
        let toward_minus = rounding == Rounding::TowardNegative;
        let negative = |a, b| ppc_sum(rounding, a, b).unwrap().negative();
        assert_eq!(negative(one, one.negated()), toward_minus, "{rounding:?}");
        assert_eq!(negative(one.negated(), one), toward_minus, "{rounding:?}");
        assert_eq!(
            negative(zero(false), zero(true)),
            toward_minus,
            "{rounding:?}"
        );
        assert!(negative(zero(true), zero(true)), "{rounding:?}");
        assert!(!negative(zero(false), zero(false)), "{rounding:?}");
        assert!(negative(one.negated(), zero(false)), "{rounding:?}");
    }
}

/// [PPC-Book1 p:97 s:4.4.1] the invalid operations: SNaN, infinity minus infinity, infinity over infinity, zero over zero, infinity times zero and the square root of a negative number.
#[test]
fn each_invalid_operation_names_its_cause() {
    const INF: u64 = 0x7FF0_0000_0000_0000;
    const NEG: u64 = 1 << 63;
    const ONE: u64 = 0x3FF0_0000_0000_0000;
    let cause = |f: fn(&mut PpcInvalid)| {
        let mut c = PpcInvalid::default();
        f(&mut c);
        c
    };
    let isi = cause(|c| c.inf_minus_inf = true);
    assert_eq!(ppc_invalid_sum(INF, NEG | INF), isi);
    assert_eq!(ppc_invalid_sum(NEG | INF, INF), isi);
    assert_eq!(ppc_invalid_sum(INF, INF), PpcInvalid::default());
    assert_eq!(ppc_invalid_sum(INF, NEG | ONE), PpcInvalid::default());
    let imz = cause(|c| c.inf_times_zero = true);
    assert_eq!(ppc_invalid_product(INF, NEG), imz);
    assert_eq!(ppc_invalid_product(0, NEG | INF), imz);
    assert_eq!(ppc_invalid_product(INF, ONE), PpcInvalid::default());
    assert_eq!(
        ppc_invalid_quotient(INF, NEG | INF),
        cause(|c| c.inf_div_inf = true)
    );
    assert_eq!(
        ppc_invalid_quotient(NEG, 0),
        cause(|c| c.zero_div_zero = true)
    );
    assert_eq!(ppc_invalid_quotient(ONE, 0), PpcInvalid::default());
    let sqrt = cause(|c| c.sqrt = true);
    assert_eq!(ppc_invalid_sqrt(NEG | ONE), sqrt);
    assert_eq!(ppc_invalid_sqrt(NEG | INF), sqrt);
    assert_eq!(ppc_invalid_sqrt(NEG | 1), sqrt);
    assert_eq!(ppc_invalid_sqrt(NEG), PpcInvalid::default());
    assert_eq!(ppc_invalid_sqrt(NEG | INF | 1 << 51), PpcInvalid::default());
    assert_eq!(
        PpcInvalid::signaling(&[ONE, INF | 1]),
        cause(|c| c.snan = true)
    );
    assert_eq!(
        PpcInvalid::signaling(&[INF | 1 << 51]),
        PpcInvalid::default()
    );
}

/// [PPC-Book1 p:97 s:4.4.1] VE = 1 leaves the target unchanged.
/// [PPC-Book1 p:98 s:4.4.2] ZE = 1 leaves the target unchanged.
#[test]
fn ve_and_ze_refuse_only_when_their_exception_occurs() {
    let invalid = PpcStatus {
        invalid: PpcInvalid {
            snan: true,
            ..PpcInvalid::default()
        },
        ..PpcStatus::default()
    };
    let zero_divide = PpcStatus {
        zx: true,
        ..PpcStatus::default()
    };
    let ve = PpcEnables {
        invalid: true,
        ..NONE
    };
    let ze = PpcEnables {
        zero_divide: true,
        ..NONE
    };
    assert_eq!(invalid.refusal(ve), Some(PpcEnabled::Invalid));
    assert_eq!(zero_divide.refusal(ze), Some(PpcEnabled::ZeroDivide));
    assert_eq!(invalid.refusal(ze), None);
    assert_eq!(zero_divide.refusal(ve), None);
    assert_eq!(invalid.refusal(NONE), None);
    assert_eq!(PpcStatus::default().refusal(ve), None);
}
