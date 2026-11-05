//! The scalar floating-point units' vector sets and the floor of each set.
//!
//! The sets come from the table-probe captures that the converter reads.
//!
//! A feature names one thing a vector exercises:
//!
//! - a source operand in one class;
//! - a rounding mode that left the result inexact;
//! - an FPSCR exception bit the word set;
//! - the record form or the plain form;
//! - a compare's outcome;
//! - the FPSCR field, bit or field mask an FPSCR move names.
//!
//! A unit's floor is the features its set must show. No expected
//! value comes from a host floating-point operation: the module reads
//! operand classes from bit patterns only.

#![deny(clippy::float_arithmetic)]

use std::collections::BTreeSet;

use super::capture::{facility_vectors, select_by, Conversion, FACILITY_PROBES};
use crate::ppu_reference::{
    word_unit, PpuReferenceFacility, PpuReferenceSet, PpuReferenceUnit, PpuReferenceVector,
    ReferenceField, PPU_REFERENCE_SET_SCHEMA_VERSION,
};

/// The scalar floating-point units this module fills: the arithmetic,
/// round, convert, compare and move units, then the FPSCR moves.
pub(super) const FP_UNITS: [&str; 38] = [
    "fadd", "fadds", "fsub", "fsubs", "fmul", "fmuls", "fdiv", "fdivs", "fmadd", "fmadds", "fmsub",
    "fmsubs", "fnmadd", "fnmadds", "fnmsub", "fnmsubs", "fsqrt", "fsqrts", "fres", "frsqrte",
    "fsel", "fmr", "fneg", "fabs", "fnabs", "frsp", "fctid", "fctidz", "fctiw", "fctiwz", "fcfid",
    "fcmpu", "fcmpo", "mcrfs", "mffs", "mtfsb0", "mtfsf", "mtfsfi",
];

/// [PPC-Book1 p:87 s:4.2.2] FPSCR bits 3 to 6 are the exception bits OX,
/// UX, ZX and XX.
const OX: u32 = 0x1000_0000;
const UX: u32 = 0x0800_0000;
const ZX: u32 = 0x0400_0000;
const XX: u32 = 0x0200_0000;
/// [PPC-Book1 p:88 s:4.2.2] FPSCR bits 7 to 12 are VXSNAN, VXISI, VXIDI,
/// VXZDZ, VXIMZ and VXVC; bit 14 is FI, and bit 22 is VXSQRT.
const VXSNAN: u32 = 0x0100_0000;
const VXISI: u32 = 0x0080_0000;
const VXIDI: u32 = 0x0040_0000;
const VXZDZ: u32 = 0x0020_0000;
const VXIMZ: u32 = 0x0010_0000;
const VXVC: u32 = 0x0008_0000;
const FI: u32 = 0x0002_0000;
const VXSQRT: u32 = 0x0000_0200;
/// [PPC-Book1 p:89 s:4.2.2] FPSCR bit 23 is VXCVI, and bits 30:31 are RN.
const VXCVI: u32 = 0x0000_0100;
const RN: u32 = 0x0000_0003;

/// An FPSCR exception bit and its name.
type Flag = (&'static str, u32);

/// A word field that names a source FPR.
#[derive(Debug, Clone, Copy)]
enum Source {
    /// Bits 11-15: FRA.
    A,
    /// Bits 16-20: FRB.
    B,
    /// Bits 21-25: FRC.
    C,
}

impl Source {
    fn name(self) -> &'static str {
        match self {
            Self::A => "fra",
            Self::B => "frb",
            Self::C => "frc",
        }
    }

    fn of(self, word: u32) -> u8 {
        let shift = match self {
            Self::A => 16,
            Self::B => 11,
            Self::C => 6,
        };
        ((word >> shift) & 31) as u8
    }
}

/// The format a unit reads its operands in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Double,
    /// [PPC-Book1 p:93 s:4.3.5.1] A single-precision word's operands must
    /// be representable in single format.
    Single,
    /// A doubleword integer, the operand of `fcfid`.
    Integer,
}

/// What a floating-point unit's floor asks of its set.
#[derive(Debug, Clone, Copy)]
pub(super) struct FpFloor {
    /// The source fields; the set shows each in every operand class.
    sources: &'static [Source],
    format: Format,
    /// Whether the unit rounds under RN; the set shows an inexact result in
    /// each mode.
    rounds: bool,
    /// The exception bits the set shows the unit setting.
    flags: &'static [Flag],
    /// Whether the word has an Rc bit; the set shows both forms.
    rc: bool,
    /// Whether the unit compares into a CR field; the set shows each
    /// outcome.
    compare: bool,
    /// The FPSCR move the unit is, if it is one.
    moves: Option<Move>,
}

/// An FPSCR move, and what its set shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Move {
    /// `mcrfs`: each source field, and a copied exception bit cleared.
    Mcrfs,
    /// `mffs`: a zero FPSCR and one with bits set.
    Mffs,
    /// `mtfsb0`: each bit.
    Mtfsb,
    /// `mtfsf`: no field, one field, several and all of them.
    Mtfsf,
    /// `mtfsfi`: each field.
    Mtfsfi,
}

/// [PPC-Book1 p:87 s:4.2.2] The FPSCR exception bits: FX, OX to VXVC, and
/// VXSOFT, VXSQRT and VXCVI.
const EXCEPTION_BITS: u32 = 0x9ff8_0700;

/// The FPSCR move floors.
///
/// [PPC-Book1 p:120 s:4.6.8] `mcrfs` copies FPSCR field BFA into CR field
/// BF and clears the exception bits it copied; `mffs` places the FPSCR in
/// FRT.
/// [PPC-Book1 p:121 s:4.6.8] `mtfsfi` writes field BF; `mtfsf` writes the
/// fields FLM names.
/// [PPC-Book1 p:122 s:4.6.8] `mtfsb0` clears bit BT.
const fn fpscr_move(moves: Move) -> FpFloor {
    FpFloor {
        rc: !matches!(moves, Move::Mcrfs),
        moves: Some(moves),
        ..plain(&[], &[])
    }
}

const AB: &[Source] = &[Source::A, Source::B];
const AC: &[Source] = &[Source::A, Source::C];
const ABC: &[Source] = &[Source::A, Source::B, Source::C];
const B: &[Source] = &[Source::B];

const ADD_FLAGS: &[Flag] = &[
    ("ox", OX),
    ("ux", UX),
    ("xx", XX),
    ("vxsnan", VXSNAN),
    ("vxisi", VXISI),
];
const MUL_FLAGS: &[Flag] = &[
    ("ox", OX),
    ("ux", UX),
    ("xx", XX),
    ("vxsnan", VXSNAN),
    ("vximz", VXIMZ),
];
const DIV_FLAGS: &[Flag] = &[
    ("ox", OX),
    ("ux", UX),
    ("zx", ZX),
    ("xx", XX),
    ("vxsnan", VXSNAN),
    ("vxidi", VXIDI),
    ("vxzdz", VXZDZ),
];
const FMA_FLAGS: &[Flag] = &[
    ("ox", OX),
    ("ux", UX),
    ("xx", XX),
    ("vxsnan", VXSNAN),
    ("vxisi", VXISI),
    ("vximz", VXIMZ),
];
const SQRT_FLAGS: &[Flag] = &[("xx", XX), ("vxsnan", VXSNAN), ("vxsqrt", VXSQRT)];
const FRES_FLAGS: &[Flag] = &[("ox", OX), ("ux", UX), ("zx", ZX), ("vxsnan", VXSNAN)];
const FRSQRTE_FLAGS: &[Flag] = &[("zx", ZX), ("vxsnan", VXSNAN), ("vxsqrt", VXSQRT)];
const FRSP_FLAGS: &[Flag] = &[("ox", OX), ("ux", UX), ("xx", XX), ("vxsnan", VXSNAN)];
const CONVERT_FLAGS: &[Flag] = &[("xx", XX), ("vxsnan", VXSNAN), ("vxcvi", VXCVI)];
const FCMPU_FLAGS: &[Flag] = &[("vxsnan", VXSNAN)];
const FCMPO_FLAGS: &[Flag] = &[("vxsnan", VXSNAN), ("vxvc", VXVC)];

const fn arith(sources: &'static [Source], format: Format, flags: &'static [Flag]) -> FpFloor {
    FpFloor {
        sources,
        format,
        rounds: true,
        flags,
        rc: true,
        compare: false,
        moves: None,
    }
}

const fn plain(sources: &'static [Source], flags: &'static [Flag]) -> FpFloor {
    FpFloor {
        sources,
        format: Format::Double,
        rounds: false,
        flags,
        rc: true,
        compare: false,
        moves: None,
    }
}

/// Each floating-point unit's floor, from the exception bits each
/// instruction alters.
///
/// [PPC-Book1 p:110 s:4.6.4] The moves alter no FPSCR bit.
/// [PPC-Book1 p:111 s:4.6.5.1] `fadd` and `fsub` set OX, UX, XX, VXSNAN
/// and VXISI.
/// [PPC-Book1 p:112 s:4.6.5.1] `fmul` sets VXIMZ where `fadd` sets VXISI;
/// `fdiv` sets ZX, VXIDI and VXZDZ as well.
/// [PPC-Book1 p:113 s:4.6.5.2] `fmadd` and `fmsub` set both VXISI and
/// VXIMZ.
/// [PPC-Book1 p:114 s:4.6.5.2] So do `fnmadd` and `fnmsub`.
/// [PPC-Book1 p:115 s:4.6.6] `frsp` sets OX, UX, XX and VXSNAN.
/// [PPC-Book1 p:116 s:4.6.6] The doubleword converts set XX, VXSNAN and
/// VXCVI; `fctidz` rounds toward zero whatever RN holds.
/// [PPC-Book1 p:117 s:4.6.6] The word converts set the same bits, and
/// `fctiwz` rounds toward zero too.
/// [PPC-Book1 p:118 s:4.6.6] `fcfid` sets XX.
/// [PPC-Book1 p:119 s:4.6.7] `fcmpu` sets VXSNAN, and `fcmpo` VXVC too.
/// [PPC-Book1 p:125 s:5.2.1.1] `fsqrt` sets XX, VXSNAN and VXSQRT; `fres`
/// sets OX, UX, ZX and VXSNAN, and leaves XX undefined.
/// [PPC-Book1 p:126 s:5.2.1.1] `frsqrte` sets ZX, VXSNAN and VXSQRT.
/// [PPC-Book1 p:126 s:5.2.2] `fsel` alters no FPSCR bit.
///
/// # Panics
///
/// Panics if `unit` is not one of [`FP_UNITS`].
pub(super) fn fp_floor(unit: &str) -> FpFloor {
    use Format::{Double as D, Single as S};
    match unit {
        "fadd" | "fsub" => arith(AB, D, ADD_FLAGS),
        "fadds" | "fsubs" => arith(AB, S, ADD_FLAGS),
        "fmul" => arith(AC, D, MUL_FLAGS),
        "fmuls" => arith(AC, S, MUL_FLAGS),
        "fdiv" => arith(AB, D, DIV_FLAGS),
        "fdivs" => arith(AB, S, DIV_FLAGS),
        "fmadd" | "fmsub" | "fnmadd" | "fnmsub" => arith(ABC, D, FMA_FLAGS),
        "fmadds" | "fmsubs" | "fnmadds" | "fnmsubs" => arith(ABC, S, FMA_FLAGS),
        "fsqrt" => arith(B, D, SQRT_FLAGS),
        "fsqrts" => arith(B, S, SQRT_FLAGS),
        "fres" => FpFloor {
            // The operand is a double, but the estimate is single
            // precision, so Single classes the operand by the single range.
            format: S,
            ..plain(B, FRES_FLAGS)
        },
        "frsqrte" => plain(B, FRSQRTE_FLAGS),
        "fsel" => plain(ABC, &[]),
        "fmr" | "fneg" | "fabs" | "fnabs" => plain(B, &[]),
        "frsp" => arith(B, D, FRSP_FLAGS),
        "fctid" | "fctiw" => arith(B, D, CONVERT_FLAGS),
        "fctidz" | "fctiwz" => plain(B, CONVERT_FLAGS),
        "fcfid" => arith(B, Format::Integer, &[("xx", XX)]),
        "fcmpu" | "fcmpo" => FpFloor {
            rc: false,
            compare: true,
            ..plain(
                AB,
                if unit == "fcmpu" {
                    FCMPU_FLAGS
                } else {
                    FCMPO_FLAGS
                },
            )
        },
        "mcrfs" => fpscr_move(Move::Mcrfs),
        "mffs" => fpscr_move(Move::Mffs),
        "mtfsb0" => fpscr_move(Move::Mtfsb),
        "mtfsf" => fpscr_move(Move::Mtfsf),
        "mtfsfi" => fpscr_move(Move::Mtfsfi),
        other => panic!("no floating-point floor for {other}"),
    }
}

/// The operand classes every source must take in a double or single
/// format.
pub(super) const CLASSES: [&str; 10] = [
    "pzero", "nzero", "pden", "nden", "pnorm", "nnorm", "pinf", "ninf", "qnan", "snan",
];

/// The integer classes `fcfid`'s source must take.
///
/// `inexact` is a value with more than 53 significant bits.
const INTEGER_CLASSES: [&str; 6] = ["zero", "one", "m1", "smax64", "smin64", "inexact"];

/// [PPC-Book1 p:90 s:4.3.1] The single format's minimum exponent is -126,
/// which the double format biases to 897.
const SINGLE_NORMAL_EXP: u64 = 897;

/// The class of a single-format word.
pub(super) fn single_class(w: u32) -> &'static str {
    let negative = w >> 31 == 1;
    let exp = (w >> 23) & 0xff;
    let frac = w & ((1 << 23) - 1);
    let signed = |p: &'static str, n: &'static str| if negative { n } else { p };
    match (exp, frac) {
        (0, 0) => signed("pzero", "nzero"),
        (0, _) => signed("pden", "nden"),
        (0xff, 0) => signed("pinf", "ninf"),
        (0xff, f) if f >> 22 == 1 => "qnan",
        (0xff, _) => "snan",
        _ => signed("pnorm", "nnorm"),
    }
}

/// The class of a double-format value; under `single`, a nonzero value
/// below the single normal range is a denormal.
pub(super) fn class(v: u64, single: bool) -> &'static str {
    let negative = v >> 63 == 1;
    let exp = (v >> 52) & 0x7ff;
    let frac = v & ((1 << 52) - 1);
    let signed = |p: &'static str, n: &'static str| if negative { n } else { p };
    match (exp, frac) {
        (0, 0) => signed("pzero", "nzero"),
        (0x7ff, 0) => signed("pinf", "ninf"),
        (0x7ff, f) if f >> 51 == 1 => "qnan",
        (0x7ff, _) => "snan",
        (0, _) => signed("pden", "nden"),
        (e, _) if single && e < SINGLE_NORMAL_EXP => signed("pden", "nden"),
        _ => signed("pnorm", "nnorm"),
    }
}

/// The class of a doubleword integer operand.
fn integer_class(v: u64) -> &'static str {
    match v {
        0 => "zero",
        1 => "one",
        u64::MAX => "m1",
        0x7fff_ffff_ffff_ffff => "smax64",
        0x8000_0000_0000_0000 => "smin64",
        _ => {
            let m = (v as i64).unsigned_abs();
            if (m >> m.trailing_zeros()) >> 53 != 0 {
                "inexact"
            } else {
                "other"
            }
        }
    }
}

/// The FPSCR value `v` expects.
///
/// # Panics
///
/// Panics if the vector does not expect an FPSCR value.
fn expected_fpscr(v: &PpuReferenceVector) -> u32 {
    match v.expected.state.fpscr {
        ReferenceField::Value { value } => value,
        _ => panic!("{}: the expected FPSCR is not a value", v.name),
    }
}

/// The CR value `v` expects.
///
/// # Panics
///
/// Panics if the vector does not expect a CR value.
fn expected_cr(v: &PpuReferenceVector) -> u32 {
    match v.expected.state.cr {
        ReferenceField::Value { value } => value,
        _ => panic!("{}: the expected CR is not a value", v.name),
    }
}

/// The features `v` shows towards `floor`, read from its first word of
/// `unit`.
pub(super) fn fp_features(floor: &FpFloor, unit: &str, v: &PpuReferenceVector) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let of_unit = |w: &u32| matches!(word_unit(*w), Some(PpuReferenceUnit::Instruction { mnemonic }) if mnemonic == unit);
    let Some(&word) = v.words.iter().find(|w| of_unit(w)) else {
        return out;
    };
    let fpr = |n: u8| v.initial_state.fpr.get(&n).copied().unwrap_or_default();
    for source in floor.sources {
        let value = fpr(source.of(word));
        let name = match floor.format {
            Format::Integer => integer_class(value),
            f => class(value, f == Format::Single),
        };
        out.insert(format!("{}:{name}", source.name()));
    }
    if floor.rc {
        out.insert(if word & 1 == 1 { "rc" } else { "norc" }.into());
    }
    // [PPC-Book1 p:87 s:4.2.2] The FPSCR exception bits are sticky and
    // other words also change them, so the start and end status of a
    // vector with several words do not show what the unit's word did.
    if v.words.len() != 1 {
        return out;
    }
    let start = v.initial_state.fpscr.unwrap_or_default();
    let end = expected_fpscr(v);
    if let Some(moves) = floor.moves {
        out.insert(move_feature(moves, word, start));
        if moves == Move::Mcrfs {
            let field = 0xf000_0000 >> (4 * ((word >> 18) & 7));
            if start & field & EXCEPTION_BITS != 0 && end & field & EXCEPTION_BITS == 0 {
                out.insert("clears".into());
            }
        }
    }
    if floor.rounds && end & FI != 0 {
        out.insert(format!("rn{}-inexact", start & RN));
    }
    for &(name, bit) in floor.flags {
        if end & bit != 0 && start & bit == 0 {
            out.insert(format!("sets:{name}"));
        }
    }
    if floor.compare {
        let field = (word >> 23) & 7;
        let bits = (expected_cr(v) >> (28 - 4 * field)) & 0xf;
        // [PPC-Book1 p:119 s:4.6.7] A compare sets one bit of the field and
        // clears the other three.
        let outcome = match bits {
            8 => "lt",
            4 => "gt",
            2 => "eq",
            1 => "un",
            other => panic!("{}: CR field {field} is {other:#x} after a compare", v.name),
        };
        out.insert(format!("cc:{outcome}"));
    }
    out
}

/// The feature an FPSCR move's word shows: the field or bit it names, the
/// kind of field mask, or whether the FPSCR it reads is zero.
fn move_feature(moves: Move, word: u32, start: u32) -> String {
    match moves {
        Move::Mcrfs => format!("bfa:{}", (word >> 18) & 7),
        Move::Mffs => if start == 0 {
            "fpscr:zero"
        } else {
            "fpscr:set"
        }
        .into(),
        Move::Mtfsb => format!("bt:{}", (word >> 21) & 31),
        Move::Mtfsf => match ((word >> 17) & 0xff).count_ones() {
            0 => "flm:none",
            1 => "flm:one",
            8 => "flm:all",
            _ => "flm:several",
        }
        .into(),
        Move::Mtfsfi => format!("bf:{}", (word >> 23) & 7),
    }
}

/// The features an FPSCR move's set must show.
fn move_required(moves: Move) -> Vec<String> {
    match moves {
        Move::Mcrfs => (0..8)
            .map(|n| format!("bfa:{n}"))
            .chain(["clears".to_owned()])
            .collect(),
        Move::Mffs => vec!["fpscr:zero".into(), "fpscr:set".into()],
        Move::Mtfsb => (0..32).map(|n| format!("bt:{n}")).collect(),
        Move::Mtfsf => ["none", "one", "several", "all"]
            .map(|k| format!("flm:{k}"))
            .into(),
        Move::Mtfsfi => (0..8).map(|n| format!("bf:{n}")).collect(),
    }
}

/// The features a floating-point unit's set must show.
pub(super) fn fp_required(floor: &FpFloor) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Some(moves) = floor.moves {
        out.extend(move_required(moves));
    }
    let classes: &[&str] = match floor.format {
        Format::Integer => &INTEGER_CLASSES,
        _ => &CLASSES,
    };
    for source in floor.sources {
        for class in classes {
            out.insert(format!("{}:{class}", source.name()));
        }
    }
    if floor.rounds {
        out.extend((0..4).map(|rn| format!("rn{rn}-inexact")));
    }
    out.extend(floor.flags.iter().map(|(name, _)| format!("sets:{name}")));
    if floor.rc {
        out.extend(["rc".to_owned(), "norc".to_owned()]);
    }
    if floor.compare {
        out.extend(["lt", "gt", "eq", "un"].map(|c| format!("cc:{c}")));
    }
    out
}

/// The `fpscr` facility's set: every case of the [`FACILITY_PROBES`]
/// captures, each over two or three words.
pub(super) fn fpscr_facility_set() -> PpuReferenceSet {
    PpuReferenceSet {
        schema_version: PPU_REFERENCE_SET_SCHEMA_VERSION,
        unit: PpuReferenceUnit::Facility {
            name: PpuReferenceFacility::Fpscr,
        },
        vectors: FACILITY_PROBES
            .iter()
            .flat_map(|probe| facility_vectors(probe))
            .collect(),
    }
}

/// The set written for floating-point `unit`, from the converter's
/// candidates.
pub(super) fn fp_set(unit: &str, conversion: &Conversion) -> PpuReferenceSet {
    let candidates = conversion.candidates.get(unit).cloned().unwrap_or_default();
    let floor = fp_floor(unit);
    let shown: Vec<BTreeSet<String>> = candidates
        .iter()
        .map(|v| fp_features(&floor, unit, v))
        .collect();
    PpuReferenceSet {
        schema_version: PPU_REFERENCE_SET_SCHEMA_VERSION,
        unit: PpuReferenceUnit::Instruction {
            mnemonic: unit.to_owned(),
        },
        vectors: select_by(&candidates, &shown),
    }
}
