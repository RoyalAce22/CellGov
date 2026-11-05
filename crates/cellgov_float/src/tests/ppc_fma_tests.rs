//! `ppc_madd`, `ppc_msub`, `ppc_nmadd` and `ppc_nmsub` against the
//! definitional oracle and the documents. The tests cover:
//!
//! - finite operand triples in every mode and both widths;
//! - one rounding of the exact result;
//! - the sign of an exact zero result and of a negated NaN;
//! - the invalid causes, NaN selection and infinities;
//! - the single-precision operand range in each position.

use super::oracle::{define, rounds_up};
use super::round_tests::{finite, product, sum};
use crate::format::{Binary32, Binary64, Format};
use crate::operand::{unpack, Operand};
use crate::ppc::{PpcInvalid, PpcPrecision, PpcResult, PpcStatus, PPC_GENERATED_NAN};
use crate::ppc_arith::{ppc_madd, ppc_msub, ppc_mul, ppc_nmadd, ppc_nmsub, ppc_sub};
use crate::round::{round_pack, Exact, Policy, Rounding};

const MODES: [Rounding; 4] = [
    Rounding::NearestEven,
    Rounding::TowardZero,
    Rounding::TowardPositive,
    Rounding::TowardNegative,
];

const NEG: u64 = 1 << 63;
const ONE: u64 = 0x3FF0_0000_0000_0000;
const INF: u64 = 0x7FF0_0000_0000_0000;
const QNAN: u64 = 0x7FF8_0000_0000_0AAA;
const SNAN: u64 = 0x7FF0_0000_0000_0BBB;

/// Finite addends and multiplicands every single-precision form computes
/// on.
const SINGLE_RANGE: [u64; 9] = [
    0,
    ONE,
    0x4008_0000_0000_0000, // 3
    0x3FF0_0000_0000_0001, // 1 + 2^-52
    0x3FF8_0000_0000_0000, // 1.5
    0x3C30_0000_0000_0000, // 2^-60
    0x37D0_0000_0000_0000, // 2^-130
    0x47EF_FFFF_E000_0000, // the largest single
    0x4630_0000_0000_0000, // 2^100
];

/// Finite operands only the double forms take: the double range's edges.
const DOUBLE_ONLY: [u64; 4] = [
    0x0000_0000_0000_0001, // the smallest denormal
    0x0010_0000_0000_0000, // the smallest normal
    0x0170_0000_0000_0000, // 2^-1000
    0x7FEF_FFFF_FFFF_FFFF, // the largest double
];

/// FRC values: fewer than FRA and FRB, so the triples stay few.
const MULTIPLIERS: [u64; 4] = [
    ONE,
    0x3FF0_0000_0000_0001, // 1 + 2^-52
    0x3FD5_5555_5555_5555, // the double nearest 1/3
    0x3C30_0000_0000_0000, // 2^-60
];

/// Each operand and its negation.
fn signed(values: &[u64]) -> Vec<u64> {
    values.iter().flat_map(|&v| [v, v | NEG]).collect()
}

/// The exact value of a finite double-format operand.
fn exact(bits: u64) -> Exact {
    finite::<Binary64>(Policy::PowerPc, bits)
        .unwrap_or_else(|| Exact::new(bits & NEG != 0, 0, 0).unwrap())
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

/// Whether the test's sum, which shifts the term with the higher exponent
/// left to the other's, holds `p + e` in 127 bits.
fn aligns(p: Exact, e: Exact) -> bool {
    let (high, gap) = if p.exponent() >= e.exponent() {
        (p, p.exponent() - e.exponent())
    } else {
        (e, e.exponent() - p.exponent())
    };
    (u128::BITS - high.significand().leading_zeros()) as i32 + gap <= 127
}

/// The exact `p + e`, with the sign an exact zero sum takes in `rounding`.
/// `None` when the terms are too far apart for the test's sum.
fn exact_sum(rounding: Rounding, p: Exact, e: Exact) -> Option<Exact> {
    match (p.significand(), e.significand()) {
        (0, 0) => {}
        (0, _) => return Some(e),
        (_, 0) => return Some(p),
        _ if !aligns(p, e) => return None,
        _ => {
            let s = sum(p, e);
            if s.significand() != 0 {
                return Some(s);
            }
        }
    }
    let negative = if p.negative() == e.negative() {
        p.negative()
    } else {
        rounding == Rounding::TowardNegative
    };
    Some(Exact::new(negative, 0, 0).unwrap())
}

/// `r` with its sign flipped.
fn negated(r: PpcResult) -> PpcResult {
    PpcResult {
        bits: r.bits ^ NEG,
        ..r
    }
}

/// [PPC-Book1 p:113 s:4.6.5.2] fmadd and fmsub round the exact (FRA x FRC) +/- FRB once to the target precision by `FPSCR[RN]`.
/// [PPC-Book1 p:114 s:4.6.5.2] fnmadd and fnmsub negate the rounded result.
/// [Aharoni2003 p:17 s:1] operand classes crossed per operand.
#[test]
fn finite_operand_triples_round_once_like_the_oracle_in_every_mode_and_width() {
    let single = signed(&SINGLE_RANGE);
    let double: Vec<u64> = single.iter().copied().chain(signed(&DOUBLE_ONLY)).collect();
    let multipliers = signed(&MULTIPLIERS);
    let mut checked = 0;
    for (precision, operands) in [
        (PpcPrecision::Single, &single),
        (PpcPrecision::Double, &double),
    ] {
        for &a in operands.iter() {
            for &b in operands.iter() {
                for &c in &multipliers {
                    let p = product(exact(a), exact(c));
                    for rounding in MODES {
                        let tag =
                            format!("{precision:?} {rounding:?} {a:#018x} {b:#018x} {c:#018x}");
                        if let Some(s) = exact_sum(rounding, p, exact(b)) {
                            let want = oracle(precision, rounding, s);
                            assert_eq!(ppc_madd(precision, rounding, a, b, c), want, "madd {tag}");
                            let got = ppc_nmadd(precision, rounding, a, b, c);
                            assert_eq!(got, negated(want), "nmadd {tag}");
                            checked += 2;
                        }
                        if let Some(s) = exact_sum(rounding, p, exact(b ^ NEG)) {
                            let want = oracle(precision, rounding, s);
                            assert_eq!(ppc_msub(precision, rounding, a, b, c), want, "msub {tag}");
                            let got = ppc_nmsub(precision, rounding, a, b, c);
                            assert_eq!(got, negated(want), "nmsub {tag}");
                            checked += 2;
                        }
                    }
                }
            }
        }
    }
    // The triples whose product and addend the test's sum aligns, or that
    // have a zero term.
    assert_eq!(checked, 49792);
}

/// [PPC-Book1 p:101 s:4.5.2] the operand bits that alignment shifts out are ORed into the X' bit, which takes part in the add and the rounding.
/// Terms too far apart for the test's sum: the addend survives only as a
/// sticky bit, which decides the directed modes.
#[test]
fn an_addend_shifted_out_entirely_still_rounds_the_result_in_every_mode() {
    const DDEN: u64 = 1;
    const BELOW_ONE: u64 = 0x3FEF_FFFF_FFFF_FFFF;
    const ABOVE_ONE: u64 = 0x3FF0_0000_0000_0001;
    let d = PpcPrecision::Double;
    let status = |fr| PpcStatus {
        fr,
        fi: true,
        ..PpcStatus::default()
    };
    for (rounding, below, above) in [
        (Rounding::NearestEven, (ONE, true), (ONE, false)),
        (Rounding::TowardZero, (BELOW_ONE, false), (ONE, false)),
        (Rounding::TowardPositive, (ONE, true), (ABOVE_ONE, true)),
        (Rounding::TowardNegative, (BELOW_ONE, false), (ONE, false)),
    ] {
        let tag = format!("{rounding:?}");
        let want = |(bits, fr)| PpcResult {
            bits,
            status: status(fr),
        };
        assert_eq!(
            ppc_msub(d, rounding, ONE, DDEN, ONE),
            want(below),
            "msub {tag}"
        );
        assert_eq!(
            ppc_madd(d, rounding, ONE, DDEN, ONE),
            want(above),
            "madd {tag}"
        );
    }
}

/// The `ppu_fp_fma_modes` console capture: (1 + 2^-27)^2 - (1 + 2^-26) is
/// 2^-54, where a product rounded first gives 0.
#[test]
fn the_product_is_not_rounded_before_the_sum() {
    const A: u64 = 0x3FF0_0000_0200_0000; // 1 + 2^-27
    const B: u64 = 0xBFF0_0000_0400_0000; // -(1 + 2^-26)
    const M54: u64 = 0x3C90_0000_0000_0000;
    let d = PpcPrecision::Double;
    let r = Rounding::NearestEven;
    let exact = PpcStatus::default();
    let rounded = ppc_mul(d, r, A, A).bits;
    assert_eq!(ppc_sub(d, r, rounded, B ^ NEG).bits, 0);
    assert_eq!(
        ppc_madd(d, r, A, B, A),
        PpcResult {
            bits: M54,
            status: exact
        }
    );
    assert_eq!(
        ppc_nmadd(d, r, A, B, A),
        PpcResult {
            bits: M54 | NEG,
            status: exact
        }
    );
    // 3 x (the double nearest 1/3) - 1 is -2^-54, exact even in single.
    let third = 0x3FD5_5555_5555_5555;
    let three = 0x4008_0000_0000_0000;
    for precision in [d, PpcPrecision::Single] {
        assert_eq!(
            ppc_msub(precision, r, three, ONE, third).bits,
            M54 | NEG,
            "{precision:?}"
        );
    }
}

/// [PPC-Book1 p:92 s:4.3.3] an exact zero sum of opposite signs is +0 in every mode except toward -infinity, where it is -0.
/// The `ppu_fp_fma_modes` console capture: fnmadd and fnmsub negate that zero.
#[test]
fn an_exact_zero_result_is_signed_by_the_mode_then_negated() {
    for precision in [PpcPrecision::Double, PpcPrecision::Single] {
        for rounding in MODES {
            let zero = if rounding == Rounding::TowardNegative {
                NEG
            } else {
                0
            };
            let tag = format!("{precision:?} {rounding:?}");
            assert_eq!(
                ppc_madd(precision, rounding, ONE, NEG | ONE, ONE).bits,
                zero,
                "{tag}"
            );
            assert_eq!(
                ppc_msub(precision, rounding, ONE, ONE, ONE).bits,
                zero,
                "{tag}"
            );
            assert_eq!(
                ppc_nmadd(precision, rounding, ONE, NEG | ONE, ONE).bits,
                zero ^ NEG,
                "{tag}"
            );
            assert_eq!(
                ppc_nmsub(precision, rounding, ONE, ONE, ONE).bits,
                zero ^ NEG,
                "{tag}"
            );
            // -0 x 1 + 0 and -0 x 1 + -0.
            assert_eq!(
                ppc_madd(precision, rounding, NEG, 0, ONE).bits,
                zero,
                "{tag}"
            );
            assert_eq!(
                ppc_madd(precision, rounding, NEG, NEG, ONE).bits,
                NEG,
                "{tag}"
            );
        }
    }
}

/// [PPC-Book1 p:97 s:4.4.1] an SNaN operand, infinity times zero and infinity minus infinity are invalid.
/// [PPC-Book1 p:113 s:4.6.5.2] the invalid-operation bits are set as if the multiply and then the add ran as two instructions.
/// [PPC-Book1 p:91 s:4.3.2] a NaN operand is the result, FRA first, then FRB, then FRC, quieted; else a disabled invalid operation gives the generated QNaN.
/// [PPC-Book1 p:114 s:4.6.5.2] the negated forms leave the sign of a NaN result.
#[test]
fn the_invalid_causes_and_nan_selection_come_before_the_negation() {
    let invalid = |f: fn(&mut PpcInvalid)| {
        let mut c = PpcInvalid::default();
        f(&mut c);
        PpcStatus {
            invalid: c,
            ..PpcStatus::default()
        }
    };
    let isi = invalid(|c| c.inf_minus_inf = true);
    let imz = invalid(|c| c.inf_times_zero = true);
    let snan = invalid(|c| c.snan = true);
    let snan_imz = invalid(|c| {
        c.snan = true;
        c.inf_times_zero = true;
    });
    let quiet = |s: u64| s | 1 << 51;
    let r = Rounding::NearestEven;
    for precision in [PpcPrecision::Double, PpcPrecision::Single] {
        let result = |bits, status| PpcResult { bits, status };
        let generated = |status| result(PPC_GENERATED_NAN, status);
        // inf x 1 + -inf, and inf x 1 - inf.
        assert_eq!(ppc_madd(precision, r, INF, NEG | INF, ONE), generated(isi));
        assert_eq!(ppc_msub(precision, r, INF, INF, ONE), generated(isi));
        assert_eq!(ppc_nmadd(precision, r, INF, NEG | INF, ONE), generated(isi));
        assert_eq!(ppc_nmsub(precision, r, INF, INF, ONE), generated(isi));
        // inf x 0 + inf of either sign: only the product's cause.
        assert_eq!(ppc_madd(precision, r, INF, NEG | INF, 0), generated(imz));
        assert_eq!(ppc_nmsub(precision, r, 0, INF, NEG | INF), generated(imz));
        // inf x 0 + QNaN: the NaN is the result, with the product's cause.
        assert_eq!(ppc_madd(precision, r, INF, QNAN, 0), result(QNAN, imz));
        assert_eq!(
            ppc_madd(precision, r, INF, SNAN, 0),
            result(quiet(SNAN), snan_imz)
        );
        // FRA, then FRB, then FRC.
        assert_eq!(
            ppc_madd(precision, r, QNAN | NEG, SNAN, QNAN),
            result(QNAN | NEG, snan)
        );
        assert_eq!(
            ppc_madd(precision, r, ONE, QNAN | NEG, SNAN),
            result(QNAN | NEG, snan)
        );
        assert_eq!(
            ppc_madd(precision, r, ONE, ONE, SNAN),
            result(quiet(SNAN), snan)
        );
        // The negated forms keep the NaN's sign; msub reads FRB's raw sign.
        let none = PpcStatus::default();
        assert_eq!(
            ppc_nmadd(precision, r, QNAN | NEG, ONE, ONE),
            result(QNAN | NEG, none)
        );
        assert_eq!(
            ppc_nmsub(precision, r, ONE, SNAN, ONE),
            result(quiet(SNAN), snan)
        );
        assert_eq!(ppc_msub(precision, r, ONE, QNAN, ONE), result(QNAN, none));
        // A NaN in FRA x FRC with an infinity FRB: no sum to check.
        assert_eq!(
            ppc_madd(precision, r, INF, NEG | INF, QNAN),
            result(QNAN, none)
        );
    }
}

/// [PPC-Book1 p:91 s:4.3.2] arithmetic on infinities is exact and signals no exception other than an invalid operation.
#[test]
fn an_infinity_operand_gives_an_exact_infinity_negated_by_the_negated_forms() {
    let exact_inf = |bits| PpcResult {
        bits,
        status: PpcStatus::default(),
    };
    for precision in [PpcPrecision::Double, PpcPrecision::Single] {
        for rounding in MODES {
            assert_eq!(
                ppc_madd(precision, rounding, INF, NEG | ONE, ONE),
                exact_inf(INF)
            );
            assert_eq!(
                ppc_madd(precision, rounding, ONE, ONE, NEG | INF),
                exact_inf(NEG | INF)
            );
            assert_eq!(
                ppc_madd(precision, rounding, NEG | 1, ONE, INF),
                exact_inf(NEG | INF)
            );
            assert_eq!(ppc_madd(precision, rounding, INF, INF, ONE), exact_inf(INF));
            assert_eq!(
                ppc_msub(precision, rounding, ONE, INF, ONE),
                exact_inf(NEG | INF)
            );
            assert_eq!(
                ppc_msub(precision, rounding, NEG | INF, INF, ONE),
                exact_inf(NEG | INF)
            );
            assert_eq!(
                ppc_nmadd(precision, rounding, ONE, INF, ONE),
                exact_inf(NEG | INF)
            );
            assert_eq!(
                ppc_nmsub(precision, rounding, INF, NEG | INF, ONE),
                exact_inf(NEG | INF)
            );
        }
    }
}

/// The `ppu_fp_fma_modes` console capture: an operand outside the single
/// exponents gives fmadds the generated QNaN with no cause. This holds for
/// the operand as FRA, FRB or FRC, unless an operand is a NaN or an
/// infinity. The `ppu_fp_fma_range_order` capture shows the same for fnmsubs.
#[test]
fn a_single_operand_outside_the_single_exponents_gives_the_generated_nan() {
    const P127: u64 = 0x47E0_0000_0000_0000;
    const P128: u64 = 0x47F0_0000_0000_0000;
    const M149: u64 = 0x36A0_0000_0000_0000;
    const M150: u64 = 0x3690_0000_0000_0000;
    const P200: u64 = 0x4C70_0000_0000_0000;
    const M200: u64 = 0x3370_0000_0000_0000;
    let s = PpcPrecision::Single;
    let r = Rounding::NearestEven;
    let generated = PpcResult {
        bits: PPC_GENERATED_NAN,
        status: PpcStatus::default(),
    };
    for out in [P128, M150, 0x0010_0000_0000_0000, NEG | P128] {
        let tag = format!("{out:#018x}");
        assert_eq!(ppc_madd(s, r, out, 0, ONE), generated, "FRA {tag}");
        assert_eq!(ppc_madd(s, r, ONE, out, ONE), generated, "FRB {tag}");
        assert_eq!(ppc_madd(s, r, ONE, 0, out), generated, "FRC {tag}");
        assert_eq!(ppc_nmsub(s, r, ONE, 0, out), generated, "FRC {tag}");
    }
    for edge in [P127, M149] {
        assert_eq!(ppc_madd(s, r, edge, 0, ONE).bits, edge);
        assert_eq!(ppc_madd(s, r, ONE, 0, edge).bits, edge);
    }
    // 0 x 2^200 + 1, and 2^200 x 2^-200 - 1: the product is in range.
    assert_eq!(ppc_madd(s, r, 0, ONE, P200), generated);
    assert_eq!(ppc_madd(s, r, P200, NEG | ONE, M200), generated);
    assert_eq!(ppc_madd(s, r, INF, P200, ONE).bits, INF);
    assert_eq!(ppc_madd(s, r, QNAN, P200, ONE).bits, QNAN);
    assert_eq!(
        ppc_madd(PpcPrecision::Double, r, P200, NEG | ONE, M200).bits,
        0
    );
}
