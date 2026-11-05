//! `ppc_add`, `ppc_sub` and `ppc_mul` against the definitional oracle and
//! the documents. The tests cover:
//!
//! - finite operand pairs in every mode and both widths;
//! - the sign of an exact zero sum;
//! - the invalid causes;
//! - infinities;
//! - NaN selection;
//! - the single-precision operand range.

use super::oracle::{define, rounds_up};
use super::round_tests::{finite, product, sum};
use crate::format::{Binary32, Binary64, Format};
use crate::operand::{unpack, Operand};
use crate::ppc::{PpcInvalid, PpcPrecision, PpcResult, PpcStatus, PPC_GENERATED_NAN};
use crate::ppc_arith::{ppc_add, ppc_mul, ppc_single_operand_out_of_range, ppc_sub};
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

/// Finite operands every single-precision arm computes on: zero, values
/// with and without bits below single precision, a single denormal held
/// as a double normal, and the largest single.
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

/// Finite operands only the double arms take: the double range's edges.
const DOUBLE_ONLY: [u64; 4] = [
    0x0000_0000_0000_0001, // the smallest denormal
    0x0010_0000_0000_0000, // the smallest normal
    0x0170_0000_0000_0000, // 2^-1000
    0x7FEF_FFFF_FFFF_FFFF, // the largest double
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

/// The exact `a + b`, with the sign an exact zero sum takes in `rounding`.
/// `None` when the operands are too far apart for the test's sum, which
/// aligns them in 128 bits.
fn exact_sum(rounding: Rounding, a: Exact, b: Exact) -> Option<Exact> {
    match (a.significand(), b.significand()) {
        (0, 0) => {}
        (0, _) => return Some(b),
        (_, 0) => return Some(a),
        _ if (a.exponent() - b.exponent()).abs() > 70 => return None,
        _ => {
            let s = sum(a, b);
            if s.significand() != 0 {
                return Some(s);
            }
        }
    }
    let negative = if a.negative() == b.negative() {
        a.negative()
    } else {
        rounding == Rounding::TowardNegative
    };
    Some(Exact::new(negative, 0, 0).unwrap())
}

/// [PPC-Book1 p:111 s:4.6.5.1] fadd and fsub round the exact sum once to the target precision by `FPSCR[RN]`.
/// [PPC-Book1 p:112 s:4.6.5.1] fmul rounds the exact product the same way.
/// [Aharoni2003 p:17 s:1] operand classes crossed per operand.
#[test]
fn finite_operand_pairs_round_like_the_oracle_in_every_mode_and_width() {
    let single = signed(&SINGLE_RANGE);
    let double: Vec<u64> = single.iter().copied().chain(signed(&DOUBLE_ONLY)).collect();
    let mut checked = 0;
    for (precision, operands) in [
        (PpcPrecision::Single, &single),
        (PpcPrecision::Double, &double),
    ] {
        for &a in operands.iter() {
            for &b in operands.iter() {
                for rounding in MODES {
                    let tag = format!("{precision:?} {rounding:?} {a:#018x} {b:#018x}");
                    let (x, y) = (exact(a), exact(b));
                    let want = oracle(precision, rounding, product(x, y));
                    assert_eq!(ppc_mul(precision, rounding, a, b), want, "mul {tag}");
                    checked += 1;
                    if let Some(s) = exact_sum(rounding, x, y) {
                        let want = oracle(precision, rounding, s);
                        assert_eq!(ppc_add(precision, rounding, a, b), want, "add {tag}");
                        checked += 1;
                    }
                    if let Some(s) = exact_sum(rounding, x, y.negated()) {
                        let want = oracle(precision, rounding, s);
                        assert_eq!(ppc_sub(precision, rounding, a, b), want, "sub {tag}");
                        checked += 1;
                    }
                }
            }
        }
    }
    // Every product, and the sums of the pairs that lie within 70 bits.
    assert_eq!(checked, 7712);
}

/// [PPC-Book1 p:92 s:4.3.3] x - x and an exact zero sum of opposite signs are +0 in every mode except toward -infinity, where they are -0.
/// The ppu_fp_addmul_modes console capture reads the same, in both widths.
#[test]
fn x_minus_x_is_negative_zero_only_toward_minus_infinity() {
    for precision in [PpcPrecision::Double, PpcPrecision::Single] {
        for rounding in MODES {
            let zero = if rounding == Rounding::TowardNegative {
                NEG
            } else {
                0
            };
            let tag = format!("{precision:?} {rounding:?}");
            assert_eq!(ppc_sub(precision, rounding, ONE, ONE).bits, zero, "{tag}");
            assert_eq!(
                ppc_add(precision, rounding, ONE, NEG | ONE).bits,
                zero,
                "{tag}"
            );
            assert_eq!(ppc_add(precision, rounding, NEG, 0).bits, zero, "{tag}");
            assert_eq!(ppc_add(precision, rounding, NEG, NEG).bits, NEG, "{tag}");
            assert_eq!(ppc_sub(precision, rounding, 0, NEG).bits, 0, "{tag}");
            assert_eq!(
                ppc_sub(precision, rounding, ONE, ONE).status,
                PpcStatus::default(),
                "{tag}"
            );
        }
    }
}

/// [PPC-Book1 p:97 s:4.4.1] infinity minus infinity and infinity times zero are invalid, and an SNaN operand is invalid.
/// [PPC-Book1 p:91 s:4.3.2] a disabled invalid operation with no NaN operand gives the generated QNaN; a NaN operand is the result, FRA first, quieted.
#[test]
fn an_invalid_operation_names_its_cause_and_a_nan_operand_wins() {
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
    let quiet = |s: u64| s | 1 << 51;
    for precision in [PpcPrecision::Double, PpcPrecision::Single] {
        let r = Rounding::NearestEven;
        let generated = |status| PpcResult {
            bits: PPC_GENERATED_NAN,
            status,
        };
        assert_eq!(ppc_add(precision, r, INF, NEG | INF), generated(isi));
        assert_eq!(ppc_sub(precision, r, NEG | INF, NEG | INF), generated(isi));
        assert_eq!(ppc_mul(precision, r, INF, NEG), generated(imz));
        assert_eq!(ppc_mul(precision, r, 0, NEG | INF), generated(imz));
        for (a, b) in [(SNAN, ONE), (ONE, SNAN), (SNAN, QNAN)] {
            let first = if a == SNAN { a } else { b };
            let want = PpcResult {
                bits: quiet(first),
                status: snan,
            };
            assert_eq!(ppc_add(precision, r, a, b), want, "{a:#x} {b:#x}");
            assert_eq!(ppc_sub(precision, r, a, b), want, "{a:#x} {b:#x}");
            assert_eq!(ppc_mul(precision, r, a, b), want, "{a:#x} {b:#x}");
        }
        let nan_result = |bits| PpcResult {
            bits,
            status: PpcStatus::default(),
        };
        assert_eq!(
            ppc_add(precision, r, QNAN | NEG, QNAN),
            nan_result(QNAN | NEG)
        );
        assert_eq!(
            ppc_sub(precision, r, ONE, QNAN | NEG),
            nan_result(QNAN | NEG)
        );
        assert_eq!(ppc_mul(precision, r, INF, QNAN), nan_result(QNAN));
    }
}

/// [PPC-Book1 p:111 s:4.6.5.1] an infinity operand of a valid add, subtract or multiply gives an exact infinity.
#[test]
fn an_infinity_operand_gives_an_exact_infinity() {
    let exact_inf = |bits| PpcResult {
        bits,
        status: PpcStatus::default(),
    };
    for precision in [PpcPrecision::Double, PpcPrecision::Single] {
        for rounding in MODES {
            assert_eq!(ppc_add(precision, rounding, INF, NEG | ONE), exact_inf(INF));
            assert_eq!(
                ppc_add(precision, rounding, ONE, NEG | INF),
                exact_inf(NEG | INF)
            );
            assert_eq!(ppc_sub(precision, rounding, ONE, INF), exact_inf(NEG | INF));
            assert_eq!(ppc_add(precision, rounding, INF, INF), exact_inf(INF));
            assert_eq!(
                ppc_mul(precision, rounding, NEG | 1, INF),
                exact_inf(NEG | INF)
            );
            assert_eq!(
                ppc_mul(precision, rounding, ONE, NEG | INF),
                exact_inf(NEG | INF)
            );
            assert_eq!(
                ppc_mul(precision, rounding, NEG | ONE, NEG | INF),
                exact_inf(INF)
            );
        }
    }
}

/// The ppu_fp_addmul_modes console capture shows each case below.
#[test]
fn a_single_operand_outside_the_single_exponents_gives_the_generated_nan() {
    const P127: u64 = 0x47E0_0000_0000_0000;
    const P128: u64 = 0x47F0_0000_0000_0000;
    const M149: u64 = 0x36A0_0000_0000_0000;
    const M150: u64 = 0x3690_0000_0000_0000;
    for (bits, out) in [
        (P127, false),
        (P128, true),
        (0x47EF_FFFF_F000_0000, false),
        (M149, false),
        (M150, true),
        (0x0010_0000_0000_0000, true),
        (1, true),
        (0, false),
        (INF, false),
        (QNAN, false),
    ] {
        for sign in [0, NEG] {
            assert_eq!(
                ppc_single_operand_out_of_range(bits | sign),
                out,
                "{:#018x}",
                bits | sign
            );
        }
    }
    let s = PpcPrecision::Single;
    let r = Rounding::NearestEven;
    let generated = PpcResult {
        bits: PPC_GENERATED_NAN,
        status: PpcStatus::default(),
    };
    assert_eq!(ppc_add(s, r, P128, ONE), generated);
    assert_eq!(ppc_sub(s, r, ONE, M150), generated);
    assert_eq!(ppc_mul(s, r, ONE, NEG | P128), generated);
    assert_eq!(ppc_mul(s, r, 0, P128), generated);
    assert_eq!(ppc_add(s, r, INF, P128).bits, INF);
    assert_eq!(ppc_add(s, r, QNAN, P128).bits, QNAN);
    assert_eq!(ppc_mul(s, r, M149, ONE).bits, M149);
    assert_eq!(ppc_add(PpcPrecision::Double, r, P128, NEG | P128).bits, 0);
    // 1 + 2^-52 minus 1: the operand's low bits take part.
    assert_eq!(
        ppc_add(s, r, 0x3FF0_0000_0000_0001, NEG | ONE).bits,
        0x3CB0_0000_0000_0000
    );
}

/// [PPC-Book1 p:111 s:4.6.5.1] the guard, round and sticky bits of the aligned smaller operand enter the sum, however far it is shifted.
/// Operands too far apart for the oracle sum: the smaller one survives
/// only as a sticky bit, which decides the directed modes.
#[test]
fn an_operand_shifted_out_entirely_still_rounds_the_sum_in_every_mode() {
    let status = |fr| PpcStatus {
        fr,
        fi: true,
        ..PpcStatus::default()
    };
    let d = PpcPrecision::Double;
    let s = PpcPrecision::Single;
    const DDEN: u64 = 1;
    const BELOW_ONE: u64 = 0x3FEF_FFFF_FFFF_FFFF;
    const ABOVE_ONE: u64 = 0x3FF0_0000_0000_0001;
    const P100: u64 = 0x4630_0000_0000_0000;
    const M130: u64 = 0x37D0_0000_0000_0000;
    const BELOW_P100_SINGLE: u64 = 0x462F_FFFF_E000_0000;
    for (rounding, below, above) in [
        (Rounding::NearestEven, (ONE, true), (ONE, false)),
        (Rounding::TowardZero, (BELOW_ONE, false), (ONE, false)),
        (Rounding::TowardPositive, (ONE, true), (ABOVE_ONE, true)),
        (Rounding::TowardNegative, (BELOW_ONE, false), (ONE, false)),
    ] {
        let tag = format!("{rounding:?}");
        // 1 - 2^-1074: a borrow from the sticky bit.
        let want = PpcResult {
            bits: below.0,
            status: status(below.1),
        };
        assert_eq!(ppc_sub(d, rounding, ONE, DDEN), want, "sub {tag}");
        assert_eq!(ppc_add(d, rounding, NEG | DDEN, ONE), want, "add {tag}");
        // 1 + 2^-1074.
        let want = PpcResult {
            bits: above.0,
            status: status(above.1),
        };
        assert_eq!(ppc_add(d, rounding, ONE, DDEN), want, "add {tag}");
        // 2^100 - 2^-130 in single precision.
        let single = if below.0 == ONE {
            P100
        } else {
            BELOW_P100_SINGLE
        };
        let want = PpcResult {
            bits: single,
            status: status(below.1),
        };
        assert_eq!(ppc_sub(s, rounding, P100, M130), want, "subs {tag}");
    }
}
