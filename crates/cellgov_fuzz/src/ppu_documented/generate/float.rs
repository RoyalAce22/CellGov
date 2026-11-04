//! The floating-point sets: `fctiw-high-word`, `mffs-high-word`,
//! `fp59-wide-operand`, `fp-overflow-fr`, `stfs-below-denormal` and
//! `fpscr-ni`.
//!
//! Every operand and result is a bit pattern read off the pages; no
//! value here comes from a host floating-point unit.

use cellgov_ps3_abi::hw::ppu_status::{
    FPSCR_FI, FPSCR_FPRF, FPSCR_FR, FPSCR_FX, FPSCR_NI, FPSCR_OX, FPSCR_VX, FPSCR_VXCVI,
    FPSCR_VXSNAN, FPSCR_XX,
};

use super::draft::{buffer, dword_at, Draft, DWORD, HI, LO, RT_PRESET};
use crate::ppu_documented::encode::{fp_a, fp_x, mffs, stfs, FpOperands};
use crate::ppu_documented::set::Set;
use crate::ppu_documented::vector::{DocumentedVector, Location};

/// [PPC-Book1 p:117 s:4.6.6] fctiw and fctiwz place the integer in `FRT[32:63]`, leave `FRT[0:31]` and FPRF undefined, set FR when the result is incremented and FI when it is inexact.
const FCTIW: &str = "PPC-Book1 p:117 s:4.6.6";
/// [PPC-Book1 p:97 s:4.4.1] an invalid convert to a 32-bit integer with VE=0 sets `FRT[32:63]` to the most positive integer for a positive operand and the most negative for a negative operand or a NaN, and FR and FI to zero.
const INVALID: &str = "PPC-Book1 p:97 s:4.4.1";
/// [PPC-Book1 p:87 s:4.2.2] FX is set when an instruction changes an exception bit from 0 to 1, and VX is the OR of the invalid-operation bits.
const SUMMARY: &str = "PPC-Book1 p:87 s:4.2.2";
/// [PPC-Book1 p:120 s:4.6.8] mffs places the FPSCR in `FRT[32:63]` and leaves `FRT[0:31]` undefined.
const MFFS: &str = "PPC-Book1 p:120 s:4.6.8";
/// [PPC-Book1 p:93 s:4.3.5.1] a single-precision operation on an operand single format cannot hold leaves the result and the status bits undefined.
const FP59: &str = "PPC-Book1 p:93 s:4.3.5.1";
/// [PPC-Book1 p:98 s:4.4.3] an overflow with OE=0 sets OX, XX and FI, stores the result the rounding mode gives, sets FPRF from it, and leaves FR undefined.
const OVERFLOW: &str = "PPC-Book1 p:98 s:4.4.3";
/// [PPC-Book1 p:106 s:4.6.3] stfs stores a value below the single denormal range as an undefined word.
const STFS: &str = "PPC-Book1 p:106 s:4.6.3";
/// [PPC-Book1 p:89 s:4.2.2] with NI=1 the results need not conform to IEEE and the other FPSCR bits may mean other things.
const NI: &str = "PPC-Book1 p:89 s:4.2.2";

/// The FPSCR status bits: 0 to 23 but the reserved bit 20.
const STATUS: u128 = 0xffff_f700;

/// The whole 32-bit FPSCR.
const WORD: u128 = 0xffff_ffff;

fn bits(v: u32) -> u128 {
    u128::from(v)
}

/// What a conversion gives: the low word, and the FPSCR bits it sets.
struct Converted {
    word: u32,
    fr: bool,
    fi: bool,
    invalid: u32,
}

/// The operands of the conversion vectors.
const CONVERT_OPERANDS: &[(&str, u64)] = &[
    ("2.5", 0x4004_0000_0000_0000),
    ("3.5", 0x400c_0000_0000_0000),
    ("7", 0x401c_0000_0000_0000),
    ("-7", 0xc01c_0000_0000_0000),
    ("-2e31", 0xc1e0_0000_0000_0000),
    ("2e31", 0x41e0_0000_0000_0000),
    ("-2e31-1", 0xc1e0_0000_0020_0000),
    ("inf", 0x7ff0_0000_0000_0000),
    ("qnan", 0x7ff8_0000_0000_0000),
    ("snan", 0x7ff4_0000_0000_0000),
];

/// The conversion of `operand` in rounding mode `rn`, read off the
/// pages: the in-range operands are integers or halves.
fn convert(operand: &str, rn: u32) -> Converted {
    let exact = |word: u32| Converted {
        word,
        fr: false,
        fi: false,
        invalid: 0,
    };
    let invalid = |word: u32, bits: u32| Converted {
        word,
        fr: false,
        fi: false,
        invalid: bits,
    };
    // 2.5 and 3.5 round down to 2 and 3, or up to 3 and 4.
    let half = |down: u32| {
        let up = match rn {
            0 => down % 2 == 1,
            2 => true,
            _ => false,
        };
        Converted {
            word: down + u32::from(up),
            fr: up,
            fi: true,
            invalid: 0,
        }
    };
    match operand {
        "2.5" => half(2),
        "3.5" => half(3),
        "7" => exact(7),
        "-7" => exact(-7i32 as u32),
        "-2e31" => exact(0x8000_0000),
        "2e31" | "inf" => invalid(0x7fff_ffff, FPSCR_VXCVI),
        "-2e31-1" | "qnan" => invalid(0x8000_0000, FPSCR_VXCVI),
        "snan" => invalid(0x8000_0000, FPSCR_VXCVI | FPSCR_VXSNAN),
        other => unreachable!("no conversion operand {other}"),
    }
}

/// One `fctiw` or `fctiwz` vector in mode `rn`.
fn fctiw(name: &str, xo: u16, operand: &str, bits_in: u64, rn: u32) -> DocumentedVector {
    let toward_zero = name == "fctiwz";
    let c = convert(operand, if toward_zero { 1 } else { rn });
    let frt = Location::Fpr(3);
    let fpscr = Location::Fpscr;
    let cite = if c.invalid != 0 { INVALID } else { FCTIW };
    let xx = if c.fi { FPSCR_XX } else { 0 };
    let vx = if c.invalid != 0 { FPSCR_VX } else { 0 };
    let fx = if c.fi || c.invalid != 0 { FPSCR_FX } else { 0 };
    let named = FPSCR_FX
        | FPSCR_VX
        | FPSCR_XX
        | FPSCR_VXSNAN
        | FPSCR_VXCVI
        | FPSCR_FR
        | FPSCR_FI
        | FPSCR_FPRF;
    Draft::new(
        Set::FctiwHighWord,
        &format!("{name}/rn{rn}/{operand}"),
        &[fp_x(name, xo, 3, 1, false)],
    )
    .input(frt, u128::from(RT_PRESET))
    .input(Location::Fpr(1), u128::from(bits_in))
    .fpscr(rn)
    .value("lo", frt, LO, u128::from(c.word), cite)
    .undefined("hi", frt, HI, FCTIW)
    .undefined("fprf", fpscr, bits(FPSCR_FPRF), cite)
    .value(
        "fr",
        fpscr,
        bits(FPSCR_FR),
        bits(if c.fr { FPSCR_FR } else { 0 }),
        cite,
    )
    .value(
        "fi",
        fpscr,
        bits(FPSCR_FI),
        bits(if c.fi { FPSCR_FI } else { 0 }),
        cite,
    )
    .value("xx", fpscr, bits(FPSCR_XX), bits(xx), FCTIW)
    .value(
        "vx-bits",
        fpscr,
        bits(FPSCR_VXSNAN | FPSCR_VXCVI),
        bits(c.invalid),
        INVALID,
    )
    .value(
        "summary",
        fpscr,
        bits(FPSCR_FX | FPSCR_VX),
        bits(fx | vx),
        SUMMARY,
    )
    .value("rest", fpscr, WORD & !bits(named), bits(rn), FCTIW)
    .finish()
}

/// `fctiw-high-word`: every operand under `fctiw` and `fctiwz` in
/// round-to-nearest, and the halves under `fctiw` in the other modes.
pub(crate) fn fctiw_high_word() -> Vec<DocumentedVector> {
    let mut out = Vec::new();
    for (name, xo) in [("fctiw", 14), ("fctiwz", 15)] {
        for &(operand, bits_in) in CONVERT_OPERANDS {
            out.push(fctiw(name, xo, operand, bits_in, 0));
        }
    }
    for rn in 1..=3 {
        for &(operand, bits_in) in &CONVERT_OPERANDS[..2] {
            out.push(fctiw("fctiw", 14, operand, bits_in, rn));
        }
    }
    out
}

/// `mffs-high-word`: `mffs` over FPSCR values with and without status
/// bits set.
pub(crate) fn mffs_high_word() -> Vec<DocumentedVector> {
    let frt = Location::Fpr(3);
    [0u32, 0x0000_0003, FPSCR_FX | FPSCR_XX | 0x1, 0x0000_00f8]
        .into_iter()
        .map(|fpscr| {
            Draft::new(Set::MffsHighWord, &format!("fpscr-{fpscr:08x}"), &[mffs(3)])
                .input(frt, u128::from(RT_PRESET))
                .fpscr(fpscr)
                .value("lo", frt, LO, bits(fpscr), MFFS)
                .undefined("hi", frt, HI, MFFS)
                .value("all", Location::Fpscr, WORD, bits(fpscr), MFFS)
                .finish()
        })
        .collect()
}

/// 1 + 2^-40: its fraction has a bit below single precision.
const LOW_BITS: u64 = 0x3ff0_0000_0000_1000;
const ONE: u64 = 0x3ff0_0000_0000_0000;
/// 2^200 and 2^-200: outside the single exponent range.
const BIG: u64 = 0x4c70_0000_0000_0000;
const TINY: u64 = 0x3370_0000_0000_0000;

/// The single-precision A-form operations.
const SINGLE_OPS: &[(&str, u8)] = &[
    ("fadds", 21),
    ("fsubs", 20),
    ("fmuls", 25),
    ("fdivs", 18),
    ("fmadds", 29),
];

/// The operands an A-form operation reads, from registers 1, 2 and 4.
fn operands(name: &str) -> FpOperands {
    match name.trim_end_matches('s') {
        "fmul" => FpOperands::AC(1, 2),
        "fmadd" => FpOperands::Fused(1, 2, 4),
        "fsqrt" => FpOperands::B(1),
        _ => FpOperands::AB(1, 2),
    }
}

/// `fp59-wide-operand`: each single-precision operation on an operand
/// with bits below single precision, and on operands outside the single
/// exponent range.
pub(crate) fn fp59_wide_operand() -> Vec<DocumentedVector> {
    let frt = Location::Fpr(3);
    let mut out = Vec::new();
    for &(name, xo) in SINGLE_OPS {
        for (case, a, b) in [("low-bits", LOW_BITS, ONE), ("exponent", BIG, TINY)] {
            out.push(
                Draft::new(
                    Set::Fp59WideOperand,
                    &format!("{name}/{case}"),
                    &[fp_a(name, 59, xo, 3, operands(name), false)],
                )
                .input(frt, u128::from(RT_PRESET))
                .input(Location::Fpr(1), u128::from(a))
                .input(Location::Fpr(2), u128::from(b))
                .input(Location::Fpr(4), 0)
                .undefined("all", frt, DWORD, FP59)
                .undefined("status", Location::Fpscr, STATUS, FP59)
                .finish(),
            );
        }
    }
    out
}

/// The result an overflow stores, by precision, sign and mode, with its
/// FPRF.
fn overflow_result(single: bool, negative: bool, rn: u32) -> (u64, u32) {
    let inf = 0x7ff0_0000_0000_0000u64;
    let max = if single {
        0x47ef_ffff_e000_0000u64
    } else {
        0x7fef_ffff_ffff_ffff
    };
    // Round toward zero, and toward the infinity opposite the sign,
    // store the largest finite number; the others store infinity.
    let finite = rn == 1 || (rn == 2 && negative) || (rn == 3 && !negative);
    let magnitude = if finite { max } else { inf };
    let sign = if negative { 1u64 << 63 } else { 0 };
    let fprf = match (finite, negative) {
        (false, false) => 0x5000,
        (false, true) => 0x9000,
        (true, false) => 0x4000,
        (true, true) => 0x8000,
    };
    (sign | magnitude, fprf)
}

/// `fp-overflow-fr`: `fmul` and `fmuls` overflowing either sign in each
/// mode.
pub(crate) fn fp_overflow_fr() -> Vec<DocumentedVector> {
    let frt = Location::Fpr(3);
    let mut out = Vec::new();
    for (name, op, operand) in [
        ("fmul", 63u8, 0x7e70_0000_0000_0000u64),
        ("fmuls", 59, 0x4630_0000_0000_0000),
    ] {
        for negative in [false, true] {
            for rn in 0..4u32 {
                let (result, fprf) = overflow_result(op == 59, negative, rn);
                let after = FPSCR_FX | FPSCR_OX | FPSCR_XX | FPSCR_FI | fprf | rn;
                let sign = if negative { "neg" } else { "pos" };
                let a = operand | if negative { 1 << 63 } else { 0 };
                out.push(
                    Draft::new(
                        Set::FpOverflowFr,
                        &format!("{name}/{sign}/rn{rn}"),
                        &[fp_a(name, op, 25, 3, FpOperands::AC(1, 2), false)],
                    )
                    .input(frt, u128::from(RT_PRESET))
                    .input(Location::Fpr(1), u128::from(a))
                    .input(Location::Fpr(2), u128::from(operand))
                    .fpscr(rn)
                    .value("all", frt, DWORD, u128::from(result), OVERFLOW)
                    .undefined("fr", Location::Fpscr, bits(FPSCR_FR), OVERFLOW)
                    .value(
                        "rest",
                        Location::Fpscr,
                        WORD & !bits(FPSCR_FR),
                        bits(after),
                        OVERFLOW,
                    )
                    .finish(),
                );
            }
        }
    }
    out
}

/// `stfs-below-denormal`: `stfs` of values below the single denormal
/// range, and of values at its edges that the pages fix.
pub(crate) fn stfs_below_denormal() -> Vec<DocumentedVector> {
    let m0 = Location::Mem(0);
    let mem = buffer(16);
    let low = dword_at(&mem, 0) & LO;
    let cases: [(&str, u64, Option<u32>); 9] = [
        ("2e-200", 0x3370_0000_0000_0000, None),
        ("-2e-200", 0xb370_0000_0000_0000, None),
        ("2e-150", 0x3690_0000_0000_0000, None),
        ("double-denormal-min", 0x0000_0000_0000_0001, None),
        ("double-denormal-max", 0x000f_ffff_ffff_ffff, None),
        ("2e-149", 0x36a0_0000_0000_0000, Some(0x0000_0001)),
        ("1.5x2e-148", 0x36b8_0000_0000_0000, Some(0x0000_0003)),
        ("2e-126", 0x3810_0000_0000_0000, Some(0x0080_0000)),
        ("-0", 0x8000_0000_0000_0000, Some(0x8000_0000)),
    ];
    cases
        .into_iter()
        .map(|(case, frs, word)| {
            let d = Draft::new(Set::StfsBelowDenormal, case, &[stfs(1, 0, 10)])
                .input(Location::Fpr(1), u128::from(frs))
                .mem(mem.clone());
            match word {
                Some(w) => d.value("hi", m0, HI, u128::from(w) << 32, STFS),
                None => d.undefined("hi", m0, HI, STFS),
            }
            .value("lo", m0, LO, low, STFS)
            .finish()
        })
        .collect()
}

/// `fpscr-ni`: each operation with NI=1 in each mode, on operands that
/// give a denormal result or take a denormal operand, and `fmul`
/// overflowing.
pub(crate) fn fpscr_ni() -> Vec<DocumentedVector> {
    let frt = Location::Fpr(3);
    let p2m1000 = 0x0170_0000_0000_0000u64;
    let p2m30 = 0x3e10_0000_0000_0000u64;
    let p2p30 = 0x41d0_0000_0000_0000u64;
    let cases: [(&str, &str, u8, u8, u64, u64); 9] = [
        ("fdiv", "denormal-result", 63, 18, p2m1000, p2p30),
        (
            "fdivs",
            "denormal-result",
            59,
            18,
            0x39b0_0000_0000_0000,
            p2p30,
        ),
        (
            "fsqrt",
            "denormal-operand",
            63,
            22,
            0x0000_0000_0000_0001,
            0,
        ),
        (
            "fsqrts",
            "denormal-operand",
            59,
            22,
            0x36a0_0000_0000_0000,
            0,
        ),
        (
            "fadd",
            "denormal-result",
            63,
            21,
            0x0018_0000_0000_0000,
            0x8010_0000_0000_0000,
        ),
        ("fmul", "denormal-result", 63, 25, p2m1000, p2m30),
        ("fmadd", "denormal-result", 63, 29, p2m1000, p2m30),
        ("frsp", "denormal-result", 63, 12, 0x3730_0000_0000_0000, 0),
        (
            "fmul",
            "overflow",
            63,
            25,
            0x7e70_0000_0000_0000,
            0x7e70_0000_0000_0000,
        ),
    ];
    let mut out = Vec::new();
    for (name, case, op, xo, a, b) in cases {
        let insn = if name == "frsp" {
            fp_x(name, u16::from(xo), 3, 1, false)
        } else {
            fp_a(name, op, xo, 3, operands(name), false)
        };
        for rn in 0..4u32 {
            out.push(
                Draft::new(
                    Set::FpscrNi,
                    &format!("{name}/{case}/rn{rn}"),
                    std::slice::from_ref(&insn),
                )
                .input(frt, u128::from(RT_PRESET))
                .input(Location::Fpr(1), u128::from(a))
                .input(Location::Fpr(2), u128::from(b))
                .input(Location::Fpr(4), 0)
                .fpscr(FPSCR_NI | rn)
                .undefined("all", frt, DWORD, NI)
                .undefined("all", Location::Fpscr, WORD, NI)
                .finish(),
            );
        }
    }
    out
}
