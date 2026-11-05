//! `ppc_convert_to_integer`, `ppc_from_integer` and `ppc_round_to_single`
//! against integer and definitional oracles and the documents.

use super::oracle::{define, rounds_up};
use crate::format::{Binary32, Binary64};
use crate::operand::{unpack, Operand};
use crate::ppc::{PpcInvalid, PpcResult, PpcStatus};
use crate::ppc_convert::{
    ppc_convert_to_integer, ppc_from_integer, ppc_round_to_single, PpcInteger,
};
use crate::round::{round_pack, Exact, Policy, Rounding};

const MODES: [Rounding; 4] = [
    Rounding::NearestEven,
    Rounding::TowardZero,
    Rounding::TowardPositive,
    Rounding::TowardNegative,
];

const NEG: u64 = 1 << 63;
const INF: u64 = 0x7FF0_0000_0000_0000;
const QNAN: u64 = 0x7FF8_0000_1234_5678;
const SNAN: u64 = 0x7FF4_0000_1234_5678;

/// The integer `n / 8` rounds to in `rounding`, by integer arithmetic,
/// with whether it is inexact and whether rounding raised its magnitude.
fn eighths(n: i64, rounding: Rounding) -> (i64, bool, bool) {
    let (whole, rest) = (n.div_euclid(8), n.rem_euclid(8));
    let up = match rounding {
        Rounding::NearestEven => rest > 4 || (rest == 4 && whole % 2 != 0),
        Rounding::TowardZero => n < 0 && rest != 0,
        Rounding::TowardPositive => rest != 0,
        Rounding::TowardNegative => false,
    };
    let value = whole + i64::from(up);
    (value, rest != 0, value.abs() > (n / 8).abs())
}

/// [PPC-Book1 p:117 s:4.6.6] fctiw rounds by `FPSCR[RN]`, fctiwz toward zero; FR is set when the fraction was incremented, FI when the result is inexact.
/// [PPC-Book1 p:116 s:4.6.6] fctid and fctidz convert the same way to 64 bits.
#[test]
fn every_eighth_from_minus_ten_to_ten_rounds_like_integer_arithmetic() {
    let mut checked = 0;
    for n in -80i64..=80 {
        let bits = (n as f64 / 8.0).to_bits();
        for rounding in MODES {
            let (value, inexact, raised) = eighths(n, rounding);
            let want = PpcResult {
                bits: value as u64,
                status: PpcStatus {
                    fr: raised,
                    fi: inexact,
                    ..PpcStatus::default()
                },
            };
            for width in [PpcInteger::Word, PpcInteger::Doubleword] {
                let got = ppc_convert_to_integer(width, rounding, bits);
                assert_eq!(got, want, "{n}/8 {rounding:?} {width:?}");
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 161 * 4 * 2);
}

/// [PPC-Book1 p:97 s:4.4.1] a rounded value outside the target range, an infinity or a NaN is an invalid convert: VXCVI, and VXSNAN for an SNaN; FR and FI are zero; the most positive integer for a positive operand, else the most negative.
/// The ppu_fp_convert_modes console capture sign-extends a 32-bit result into the high word.
#[test]
fn a_value_past_either_end_of_the_range_is_an_invalid_convert() {
    let valid = |bits| PpcResult {
        bits,
        status: PpcStatus::default(),
    };
    let invalid = |bits, snan| PpcResult {
        bits,
        status: PpcStatus {
            invalid: PpcInvalid {
                convert: true,
                snan,
                ..PpcInvalid::default()
            },
            ..PpcStatus::default()
        },
    };
    let inexact = |bits, fr| PpcResult {
        bits,
        status: PpcStatus {
            fr,
            fi: true,
            ..PpcStatus::default()
        },
    };
    let (w, d) = (PpcInteger::Word, PpcInteger::Doubleword);
    let (rn, rz, rm) = (
        Rounding::NearestEven,
        Rounding::TowardZero,
        Rounding::TowardNegative,
    );
    const MAX_W: u64 = 0x7FFF_FFFF;
    const MIN_W: u64 = 0xFFFF_FFFF_8000_0000;
    const MAX_D: u64 = 0x7FFF_FFFF_FFFF_FFFF;
    const MIN_D: u64 = 0x8000_0000_0000_0000;
    // 2^31 - 0.5 rounds to 2^31 to nearest, and to 2^31 - 1 toward zero.
    assert_eq!(
        ppc_convert_to_integer(w, rn, 0x41DF_FFFF_FFE0_0000),
        invalid(MAX_W, false)
    );
    assert_eq!(
        ppc_convert_to_integer(w, rz, 0x41DF_FFFF_FFE0_0000),
        inexact(MAX_W, false)
    );
    // -2^31 - 0.5 rounds to -2^31 to nearest, and past it toward -inf.
    assert_eq!(
        ppc_convert_to_integer(w, rn, 0xC1E0_0000_0010_0000),
        inexact(MIN_W, false)
    );
    assert_eq!(
        ppc_convert_to_integer(w, rm, 0xC1E0_0000_0010_0000),
        invalid(MIN_W, false)
    );
    assert_eq!(
        ppc_convert_to_integer(w, rn, 0xC1E0_0000_0000_0000),
        valid(MIN_W)
    );
    assert_eq!(
        ppc_convert_to_integer(w, rn, 0x41E0_0000_0000_0000),
        invalid(MAX_W, false)
    );
    // 2^63, -2^63, the double below 2^63, and the double below -2^63.
    assert_eq!(
        ppc_convert_to_integer(d, rn, 0x43E0_0000_0000_0000),
        invalid(MAX_D, false)
    );
    assert_eq!(
        ppc_convert_to_integer(d, rn, 0xC3E0_0000_0000_0000),
        valid(MIN_D)
    );
    assert_eq!(
        ppc_convert_to_integer(d, rn, 0x43DF_FFFF_FFFF_FFFF),
        valid(0x7FFF_FFFF_FFFF_FC00)
    );
    assert_eq!(
        ppc_convert_to_integer(d, rn, 0xC3E0_0000_0000_0001),
        invalid(MIN_D, false)
    );
    // The largest double, the infinities and the NaNs.
    for (width, max, min) in [(w, MAX_W, MIN_W), (d, MAX_D, MIN_D)] {
        assert_eq!(
            ppc_convert_to_integer(width, rn, 0x7FEF_FFFF_FFFF_FFFF),
            invalid(max, false)
        );
        assert_eq!(ppc_convert_to_integer(width, rn, INF), invalid(max, false));
        assert_eq!(
            ppc_convert_to_integer(width, rn, NEG | INF),
            invalid(min, false)
        );
        assert_eq!(ppc_convert_to_integer(width, rn, QNAN), invalid(min, false));
        assert_eq!(
            ppc_convert_to_integer(width, rn, NEG | QNAN),
            invalid(min, false)
        );
        assert_eq!(ppc_convert_to_integer(width, rn, SNAN), invalid(min, true));
        assert_eq!(ppc_convert_to_integer(width, rn, NEG), valid(0));
        // 2^-1074 toward +inf and -2^-1074 toward -inf: 1 and -1.
        assert_eq!(
            ppc_convert_to_integer(width, Rounding::TowardPositive, 1),
            inexact(1, true)
        );
        assert_eq!(
            ppc_convert_to_integer(width, rm, NEG | 1),
            inexact(u64::MAX, true)
        );
    }
}

/// The oracle's double for the integer `value` in `rounding`.
fn double_oracle(rounding: Rounding, value: i64) -> PpcResult {
    let x = Exact::new(value < 0, u128::from(value.unsigned_abs()), 0).unwrap();
    let want = define::<Binary64>(Policy::PowerPc, rounding, x);
    PpcResult {
        bits: want.bits,
        status: PpcStatus {
            fr: rounds_up::<Binary64>(x, want.bits),
            fi: want.flags.inexact,
            ..PpcStatus::default()
        },
    }
}

/// [PPC-Book1 p:118 s:4.6.6] fcfid rounds the 64-bit signed integer in FRB once by `FPSCR[RN]`.
/// [PPC-Book1 p:93 s:4.3.5.2] only an inexact result can occur.
#[test]
fn every_integer_converts_to_the_double_the_oracle_rounds() {
    let mut values = vec![0, 1, -1, i64::MAX, i64::MIN, i64::MIN + 1];
    for shift in [52, 53, 54, 60, 62] {
        for low in [1i64, 2, 3, 5, 0x7FF] {
            values.push((1i64 << shift) + low);
            values.push(-((1i64 << shift) + low));
        }
    }
    let mut checked = 0;
    for &value in &values {
        for rounding in MODES {
            let want = double_oracle(rounding, value);
            assert_eq!(
                ppc_from_integer(rounding, value as u64),
                want,
                "{value:#x} {rounding:?}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 56 * 4);
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

/// [PPC-Book1 p:115 s:4.6.6] frsp rounds FRB once to single precision by `FPSCR[RN]`.
/// [PPC-Book1 p:139 s:A.1] an overflow gives an infinity or the largest single by mode and sign.
#[test]
fn every_finite_double_rounds_to_the_single_the_oracle_gives() {
    let operands: [u64; 12] = [
        0x3FF0_0000_1000_0000, // 1 + 2^-24, a tie
        0x3FF0_0000_1000_1000, // just above the tie
        0x3FF0_0000_3000_0000, // 1 + 3 x 2^-24, a tie
        0x7FEF_FFFF_FFFF_FFFF, // the largest double
        0x47EF_FFFF_F000_0000, // the largest single + half an ulp
        0x47EF_FFFF_E000_0000, // the largest single
        0x3690_0000_0000_0000, // 2^-150
        0x36A8_0000_0000_0000, // 1.5 x 2^-149
        0x380F_FFFF_F000_0000, // just below 2^-126, between denormals
        0x0000_0000_0000_0001, // the smallest double denormal
        0x3810_0000_0000_0000, // 2^-126
        0x4009_21FB_5444_2D18, // pi
    ];
    let mut checked = 0;
    for bits in operands {
        for sign in [0, NEG] {
            for rounding in MODES {
                let b = bits | sign;
                let Operand::Finite(x) = unpack::<Binary64>(Policy::PowerPc, b).0 else {
                    unreachable!("a finite operand")
                };
                let want = define::<Binary32>(Policy::PowerPc, rounding, x);
                let status = PpcStatus {
                    fr: want.flags.overflow || rounds_up::<Binary32>(x, want.bits),
                    fi: want.flags.inexact,
                    ox: want.flags.overflow,
                    ux: want.flags.underflow,
                    ..PpcStatus::default()
                };
                let got = ppc_round_to_single(rounding, b);
                let want = PpcResult {
                    bits: widen(want.bits),
                    status,
                };
                assert_eq!(got, want, "{b:#018x} {rounding:?}");
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 12 * 2 * 4);
}

/// [PPC-Book1 p:140 s:A.1] frsp of a QNaN gives FRB[0:34] || 0^29; an SNaN sets VXSNAN and gives the same with the quiet bit set.
/// The ppu_fp_convert_modes console capture reads both, and a zero or an infinity unchanged.
#[test]
fn a_nan_loses_its_low_29_bits_and_a_zero_or_infinity_is_kept() {
    let r = Rounding::TowardPositive;
    let none = PpcStatus::default();
    let snan = PpcStatus {
        invalid: PpcInvalid {
            snan: true,
            ..PpcInvalid::default()
        },
        ..PpcStatus::default()
    };
    let result = |bits, status| PpcResult { bits, status };
    let payload = 0x7FF8_0000_E000_0000;
    assert_eq!(
        ppc_round_to_single(r, QNAN),
        result(0x7FF8_0000_0000_0000, none)
    );
    assert_eq!(
        ppc_round_to_single(r, payload | 0x1234),
        result(payload, none)
    );
    assert_eq!(
        ppc_round_to_single(r, SNAN),
        result(0x7FFC_0000_0000_0000, snan)
    );
    assert_eq!(
        ppc_round_to_single(r, NEG | SNAN),
        result(NEG | 0x7FFC_0000_0000_0000, snan)
    );
    for kept in [0, NEG, INF, NEG | INF] {
        assert_eq!(ppc_round_to_single(r, kept), result(kept, none));
    }
    assert!(
        ppc_round_to_single(r, 0x47F0_0000_0000_0000).status.ox,
        "2^128 overflows single"
    );
}
