//! The PowerPC scalar rules against three console captures: the
//! `ppu_fp_scalar_rules` probe, the overflow rows of
//! `ppu_reference_vectors` and the NaN cases of `ppu_granule_nan`. Each
//! test reads the operands and the console's answer from the committed
//! console capture and asks the rules for the same answer.

use crate::arith::mul;
use crate::format::Binary64;
use crate::operand::{unpack, Operand};
use crate::ppc::{
    ppc_frsp_nan, ppc_invalid_product, ppc_invalid_quotient, ppc_invalid_sqrt, ppc_invalid_sum,
    ppc_is_nan, ppc_nan_result, ppc_round, ppc_sum, PpcEnables, PpcInvalid, PpcPrecision,
    PpcStatus, PPC_GENERATED_NAN,
};
use crate::round::{Exact, Policy, Rounding};
use std::path::PathBuf;

/// [PPC-Book1 p:87 s:4.2.2] FPSCR bit 3 is OX and bit 4 is UX.
const OX: u32 = 1 << (31 - 3);
const UX: u32 = 1 << (31 - 4);
/// [PPC-Book1 p:88 s:4.2.2] bits 7 to 11 are VXSNAN, VXISI, VXIDI, VXZDZ and VXIMZ, 13 is FR, 14 is FI and 22 is VXSQRT.
const VXSNAN: u32 = 1 << (31 - 7);
const VXISI: u32 = 1 << (31 - 8);
const VXIDI: u32 = 1 << (31 - 9);
const VXZDZ: u32 = 1 << (31 - 10);
const VXIMZ: u32 = 1 << (31 - 11);
const FR: u32 = 1 << (31 - 13);
const FI: u32 = 1 << (31 - 14);
const VXSQRT: u32 = 1 << (31 - 22);

const SIGN: u64 = 1 << 63;
const PROFILE: &str = "cech20-cex-493";

/// The payload of `probe`'s committed console capture, after the 8-byte
/// CGOV prefix.
fn payload(probe: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(probe)
        .join("ps3")
        .join(PROFILE)
        .join("cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "{}", path.display());
    frame[8..].to_vec()
}

fn words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().unwrap()))
        .collect()
}

fn pair(w: &[u32], at: usize) -> u64 {
    u64::from(w[at]) << 32 | u64::from(w[at + 1])
}

/// [PPC-Book1 p:89 s:4.2.2] RN 0 rounds to nearest, 1 toward zero, 2 toward +infinity, 3 toward -infinity.
fn rounding(rn: u32) -> Rounding {
    match rn & 3 {
        0 => Rounding::NearestEven,
        1 => Rounding::TowardZero,
        2 => Rounding::TowardPositive,
        _ => Rounding::TowardNegative,
    }
}

/// A zero or finite double as an exact value; `None` for an infinity or
/// a NaN.
fn exact(bits: u64) -> Option<Exact> {
    match unpack::<Binary64>(Policy::PowerPc, bits).0 {
        Operand::Zero { negative } => Exact::new(negative, 0, 0),
        Operand::Finite(x) => Some(x),
        _ => None,
    }
}

/// FR, FI, OX and UX of `status` as FPSCR bits.
fn rounding_bits(status: PpcStatus) -> u32 {
    [
        (status.fr, FR),
        (status.fi, FI),
        (status.ox, OX),
        (status.ux, UX),
    ]
    .into_iter()
    .filter(|&(set, _)| set)
    .fold(0, |acc, (_, bit)| acc | bit)
}

/// The VX bits of `invalid` as FPSCR bits.
fn invalid_bits(invalid: PpcInvalid) -> u32 {
    [
        (invalid.snan, VXSNAN),
        (invalid.inf_minus_inf, VXISI),
        (invalid.inf_div_inf, VXIDI),
        (invalid.zero_div_zero, VXZDZ),
        (invalid.inf_times_zero, VXIMZ),
        (invalid.sqrt, VXSQRT),
    ]
    .into_iter()
    .filter(|&(set, _)| set)
    .fold(0, |acc, (_, bit)| acc | bit)
}

const NONE: PpcEnables = PpcEnables {
    invalid: false,
    overflow: false,
    underflow: false,
    zero_divide: false,
    inexact: false,
};

/// One `ppu_fp_scalar_rules` case: the operation number `ppu/main.c`
/// gives, the rounding and the operands.
struct Case {
    op: u32,
    rounding: Rounding,
    fra: u64,
    frb: u64,
    frc: u64,
}

/// What the rules predict for one case.
struct Predicted {
    /// The result, when the rules decide it.
    bits: Option<u64>,
    /// FR, FI, OX and UX, when the rules round a result.
    rounded: Option<u32>,
    /// The VX bits.
    invalid: u32,
}

/// The rules' answer for `case`. Operation numbers follow `ppu/main.c`:
///
/// - 1 fadd, 2 fsub, 3 fmul, 4 fdiv, 5 fsqrt
/// - 6 fmadd
/// - 7 fadds, 8 fmuls, 9 frsp
fn predict(case: &Case) -> Predicted {
    let (a, b, c) = (case.fra, case.frb, case.frc);
    let (operands, cause): (Vec<Option<u64>>, PpcInvalid) = match case.op {
        1 | 7 => (vec![Some(a), Some(b), None], ppc_invalid_sum(a, b)),
        2 => (vec![Some(a), Some(b), None], ppc_invalid_sum(a, b ^ SIGN)),
        3 | 8 => (vec![Some(a), None, Some(c)], ppc_invalid_product(a, c)),
        4 => (vec![Some(a), Some(b), None], ppc_invalid_quotient(a, b)),
        5 => (vec![None, Some(b), None], ppc_invalid_sqrt(b)),
        6 => (vec![Some(a), Some(b), Some(c)], ppc_invalid_product(a, c)),
        9 => (vec![None, Some(b), None], PpcInvalid::default()),
        op => panic!("operation {op} is not one ppu/main.c names"),
    };
    let read: Vec<u64> = operands.iter().flatten().copied().collect();
    let invalid = PpcInvalid::signaling(&read).or(cause);
    let invalid_word = invalid_bits(invalid);
    if case.op == 9 && ppc_is_nan(b) {
        return Predicted {
            bits: Some(ppc_frsp_nan(b)),
            rounded: None,
            invalid: invalid_word,
        };
    }
    if let Some(nan) = ppc_nan_result(operands[0], operands[1], operands[2]) {
        return Predicted {
            bits: Some(nan),
            rounded: None,
            invalid: invalid_word,
        };
    }
    if invalid.any() {
        return Predicted {
            bits: Some(PPC_GENERATED_NAN),
            rounded: None,
            invalid: invalid_word,
        };
    }
    let precision = if matches!(case.op, 7 | 8) {
        PpcPrecision::Single
    } else {
        PpcPrecision::Double
    };
    let value = match case.op {
        1 | 7 => exact(a)
            .zip(exact(b))
            .map(|(x, y)| ppc_sum(case.rounding, x, y)),
        2 => exact(a)
            .zip(exact(b))
            .map(|(x, y)| ppc_sum(case.rounding, x, y.negated())),
        3 | 8 => exact(a).zip(exact(c)).map(|(x, y)| mul(x, y)),
        _ => None,
    }
    .flatten();
    match value {
        Some(x) => {
            let r = ppc_round(precision, case.rounding, x, NONE).unwrap();
            Predicted {
                bits: Some(r.bits),
                rounded: Some(rounding_bits(r.status)),
                invalid: invalid_word,
            }
        }
        None => Predicted {
            bits: None,
            rounded: None,
            invalid: invalid_word,
        },
    }
}

#[test]
fn the_rules_match_the_ppu_fp_scalar_rules_capture() {
    let w = words(&payload("ppu_fp_scalar_rules"));
    assert_eq!((w[0], w[1]), (0, 0), "the probe stopped at a step");
    let cases = w[2] as usize;
    assert_eq!(cases, 51);
    assert_eq!(w.len(), 3 + 11 * cases);
    let (mut results, mut rounded) = (0, 0);
    for k in 0..cases {
        let at = 3 + 11 * k;
        let case = Case {
            op: w[at],
            rounding: rounding(w[at + 1]),
            fra: pair(&w, at + 2),
            frb: pair(&w, at + 4),
            frc: pair(&w, at + 6),
        };
        let (frt, fpscr) = (pair(&w, at + 8), w[at + 10]);
        let want = predict(&case);
        let tag = format!(
            "case {k}: op {} {:?} {:#018x} {:#018x} {:#018x} -> {frt:#018x} fpscr {fpscr:#010x}",
            case.op, case.rounding, case.fra, case.frb, case.frc
        );
        assert_eq!(
            fpscr & (VXSNAN | VXISI | VXIDI | VXZDZ | VXIMZ | VXSQRT),
            want.invalid,
            "{tag}"
        );
        if let Some(bits) = want.bits {
            assert_eq!(frt, bits, "{tag}");
            results += 1;
        }
        if let Some(bits) = want.rounded {
            assert_eq!(fpscr & (FR | FI | OX | UX), bits, "{tag}");
            rounded += 1;
        }
    }
    assert_eq!((results, rounded), (48, 28));
}

/// [PPC-Book1 p:98 s:4.4.3] a disabled overflow delivers infinity or the largest finite number by mode and sign and sets OX and FI.
/// The page leaves FR undefined; the ppu_reference_vectors console capture settles it.
#[test]
fn a_disabled_overflow_matches_the_ppu_reference_vectors_capture() {
    let table = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_reference_vectors/cases.tsv");
    let table =
        std::fs::read_to_string(&table).unwrap_or_else(|e| panic!("{}: {e}", table.display()));
    let frame = payload("ppu_reference_vectors");
    let mut checked = 0;
    for (index, row) in table.lines().skip(1).enumerate() {
        let fields: Vec<&str> = row.split('\t').collect();
        if !fields[0].starts_with("fp-overflow-fr/") {
            continue;
        }
        let input = |name: &str| {
            fields[3]
                .split(',')
                .find_map(|kv| kv.strip_prefix(name)?.strip_prefix('='))
                .map(|v| u64::from_str_radix(v.trim_start_matches("0x"), 16).unwrap())
                .unwrap_or_else(|| panic!("{}: no {name}", fields[0]))
        };
        let precision = match fields[1].split(' ').next() {
            Some("fmul") => PpcPrecision::Double,
            Some("fmuls") => PpcPrecision::Single,
            other => panic!("{}: {other:?}", fields[0]),
        };
        let x = mul(exact(input("f1")).unwrap(), exact(input("f2")).unwrap()).unwrap();
        let want = ppc_round(precision, rounding(input("fpscr") as u32), x, NONE).unwrap();
        for run in 0..2 {
            let at = 16 + 128 * index + 64 * run;
            let f3 = u64::from_be_bytes(frame[at..at + 8].try_into().unwrap());
            let fpscr = u32::from_be_bytes(frame[at + 52..at + 56].try_into().unwrap());
            assert_eq!(f3, want.bits, "{} run {run}", fields[0]);
            assert_eq!(
                fpscr & (FR | FI | OX | UX),
                rounding_bits(want.status),
                "{} run {run}",
                fields[0]
            );
        }
        checked += 1;
    }
    assert_eq!(checked, 16);
}

/// The constants are the operands that `ppu/main.c` of the ppu_granule_nan
/// probe loads.
#[test]
fn nan_results_match_the_ppu_granule_nan_capture() {
    const ONE: u64 = 0x3FF0_0000_0000_0000;
    const QNAN_A: u64 = 0x7FF8_0000_0000_0AAA;
    const QNAN_C: u64 = 0xFFF8_0000_0000_0BBB;
    const SNAN_B: u64 = 0x7FF0_0000_0000_0123;
    let w = words(&payload("ppu_granule_nan"));
    assert_eq!((w[0], w[1]), (0, 0), "the probe stopped at a step");
    let result = |c: usize| pair(&w, 2 + 2 * c);
    for c in 0..10 {
        assert_eq!(result(c), PPC_GENERATED_NAN, "case {c}");
    }
    let propagated = [
        (10, ppc_nan_result(Some(ONE), Some(SNAN_B), Some(ONE))),
        (11, ppc_nan_result(Some(SNAN_B), Some(ONE), None)),
        (12, ppc_nan_result(Some(QNAN_C), Some(QNAN_A), None)),
        (13, ppc_nan_result(Some(QNAN_A), Some(QNAN_C), None)),
    ];
    for (c, want) in propagated {
        assert_eq!(Some(result(c)), want, "case {c}");
    }
}
