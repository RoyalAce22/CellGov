//! The vector floating-point units' sets and the floor of each set: the
//! arithmetic, compares, conversions, rounds and estimates.
//!
//! A feature names one thing a vector shows. It comes from the vector's
//! first word of the unit, its start VRs and its VSCR before that word.
//! An operand's class comes from its bit pattern alone. The
//! `ppu_vscr_mode` capture found the graphics rounding mode off and fixed,
//! so the floor has one rounding state to show. Every expected value comes
//! from the console capture; no host floating-point operation computes one.

#![deny(clippy::float_arithmetic)]

use std::collections::BTreeSet;

use super::capture::{select_by, Conversion, NJ};
use super::vmx::{field, lanes, mnemonic, sat_feature, start_vr, vscr_before};
use crate::ppu_reference::{
    PpuReferenceSet, PpuReferenceUnit, PpuReferenceVector, ReferenceField,
    PPU_REFERENCE_SET_SCHEMA_VERSION,
};

/// The vector floating-point units this module fills.
pub(super) const VMX_FLOAT_UNITS: [&str; 22] = [
    "vaddfp",
    "vsubfp",
    "vmaddfp",
    "vnmsubfp",
    "vmaxfp",
    "vminfp",
    "vcmpbfp",
    "vcmpeqfp",
    "vcmpgefp",
    "vcmpgtfp",
    "vcfsx",
    "vcfux",
    "vctsxs",
    "vctuxs",
    "vrfim",
    "vrfin",
    "vrfip",
    "vrfiz",
    "vexptefp",
    "vlogefp",
    "vrefp",
    "vrsqrtefp",
];

/// The single-precision classes an operand lane can hold.
const CLASSES: [&str; 5] = ["zero", "denormal", "normal", "infinity", "nan"];

/// The class of single-precision `bits`.
///
/// [AltiVec-PEM p:3-11 s:3.2.1] The vector unit uses the IEEE single
/// format; NJ = 1 reads a denormal operand as a zero of the same sign.
fn class(bits: u32) -> &'static str {
    let (exponent, fraction) = ((bits >> 23) & 0xff, bits & 0x7f_ffff);
    match (exponent, fraction) {
        (0, 0) => "zero",
        (0, _) => "denormal",
        (0xff, 0) => "infinity",
        (0xff, _) => "nan",
        _ => "normal",
    }
}

/// What a vector floating-point unit's floor asks of its set, besides each
/// operand's classes under NJ clear and set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FloatFloor {
    /// Nothing more.
    Classes,
    /// The plain form, and the record form with each CR6 outcome.
    Compare,
    /// A float to integer conversion: each scale, each saturation class
    /// of the scaled lane, and SAT before and after.
    ToInteger { signed: bool },
    /// An integer to float conversion: each scale, a zero lane, a lane
    /// that rounds, and SAT clear and set before the word.
    ///
    /// A signed conversion also asks for a negative lane. The conversion
    /// reads no float operand.
    FromInteger { signed: bool },
}

/// Each vector floating-point unit's floor, and the word fields of its
/// float operands.
///
/// [AltiVec-PEM p:6-31 s:6.2] `vaddfp` adds vA and vB.
/// [AltiVec-PEM p:6-71 s:6.2] `vmaddfp` multiplies vA by vC and adds vB.
/// [AltiVec-PEM p:6-72 s:6.2] `vmaxfp` compares vA and vB.
/// [AltiVec-PEM p:6-51 s:6.2] `vcmpbfp` sets CR6 when Rc = 1.
/// [AltiVec-PEM p:6-49 s:6.2] `vcfsx` converts each signed word of vB and
/// divides it by 2 to the power UIMM.
/// [AltiVec-PEM p:6-65 s:6.2] `vctsxs` multiplies each vB element by 2 to
/// the power UIMM, saturates it to a signed word and sets SAT when one
/// saturates.
/// [AltiVec-PEM p:6-66 s:6.2] `vctuxs` saturates to an unsigned word.
/// [AltiVec-PEM p:6-126 s:6.2] `vrfin` rounds each vB element.
/// [AltiVec-PEM p:6-123 s:6.2] `vrefp` estimates each vB reciprocal.
/// [AltiVec-PEM p:6-67 s:6.2] `vexptefp` estimates 2 to the power of each
/// vB element.
///
/// # Panics
///
/// Panics if `unit` is not one of [`VMX_FLOAT_UNITS`].
fn float_floor(unit: &str) -> (FloatFloor, &'static [(&'static str, u32)]) {
    const AB: &[(&str, u32)] = &[("a", 16), ("b", 11)];
    const ABC: &[(&str, u32)] = &[("a", 16), ("b", 11), ("c", 6)];
    const B: &[(&str, u32)] = &[("b", 11)];
    match unit {
        "vaddfp" | "vsubfp" | "vmaxfp" | "vminfp" => (FloatFloor::Classes, AB),
        "vmaddfp" | "vnmsubfp" => (FloatFloor::Classes, ABC),
        "vcmpbfp" | "vcmpeqfp" | "vcmpgefp" | "vcmpgtfp" => (FloatFloor::Compare, AB),
        "vcfsx" => (FloatFloor::FromInteger { signed: true }, &[]),
        "vcfux" => (FloatFloor::FromInteger { signed: false }, &[]),
        "vctsxs" => (FloatFloor::ToInteger { signed: true }, B),
        "vctuxs" => (FloatFloor::ToInteger { signed: false }, B),
        "vrfim" | "vrfin" | "vrfip" | "vrfiz" | "vexptefp" | "vlogefp" | "vrefp" | "vrsqrtefp" => {
            (FloatFloor::Classes, B)
        }
        other => panic!("no vector floating-point floor for {other}"),
    }
}

/// The words of a VR, high word first.
fn words(v: u128) -> Vec<u32> {
    lanes(v, 4, false).into_iter().map(|x| x as u32).collect()
}

/// The saturation class of float lane `bits` scaled by 2 to the power
/// `scale` and converted to a signed or unsigned word, read from its
/// exponent.
fn to_integer_class(bits: u32, scale: u32, signed: bool) -> &'static str {
    let negative = bits >> 31 == 1;
    let unbiased = ((bits >> 23) & 0xff) as i32 - 127;
    let magnitude = match class(bits) {
        "nan" => return "lane:nan",
        "zero" | "denormal" => return "lane:inside",
        // An infinity is past every word.
        "infinity" => i32::MAX,
        _ => unbiased + scale as i32,
    };
    // The scaled lane's magnitude lies in [2^magnitude, 2^(magnitude+1)).
    // A signed word holds -2^31 exactly.
    let exactly_low = magnitude == 31 && bits & 0x7f_ffff == 0;
    match (signed, negative) {
        (true, false) if magnitude >= 31 => "lane:above",
        (true, true) if magnitude >= 31 && !exactly_low => "lane:below",
        (false, false) if magnitude >= 32 => "lane:above",
        (false, true) if magnitude >= 0 => "lane:below",
        _ => "lane:inside",
    }
}

/// The classes of integer lane `x` for an integer to float conversion.
fn from_integer_class(x: u32, signed: bool) -> Vec<&'static str> {
    let mut out = Vec::new();
    if x == 0 {
        out.push("lane:zero");
    }
    let negative = signed && x >> 31 == 1;
    if negative {
        out.push("lane:negative");
    }
    let magnitude = if negative { x.wrapping_neg() } else { x };
    // A single significand holds 24 bits.
    if magnitude != 0 && 32 - magnitude.leading_zeros() - magnitude.trailing_zeros() > 24 {
        out.push("lane:rounds");
    }
    out
}

/// The CR6 outcome a record-form compare of `v` ends with.
///
/// [AltiVec-PEM p:6-53 s:6.2] A recording compare sets CR6 to all true, a
/// zero, none true and a zero.
/// [AltiVec-PEM p:6-51 s:6.2] `vcmpbfp.` sets CR6 to 0b00, all within
/// bounds and a zero.
///
/// Any other CR6, such as the seeded value left unwritten, shows no
/// required outcome.
fn cr6_feature(unit: &str, v: &PpuReferenceVector) -> &'static str {
    let ReferenceField::Value { value: cr } = v.expected.state.cr else {
        panic!("{}: the expected CR is not a value", v.name);
    };
    match ((cr >> 4) & 0xf, unit == "vcmpbfp") {
        (0x2, true) => "cr6:within",
        (0x0, true) => "cr6:outside",
        (0x8, false) => "cr6:all",
        (0x2, false) => "cr6:none",
        (0x0, false) => "cr6:mixed",
        _ => "cr6:unwritten",
    }
}

/// The features `v` shows for the floor of `unit`, read from the first
/// `unit` word of `v`.
fn float_features(unit: &str, v: &PpuReferenceVector) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Some(at) = v
        .words
        .iter()
        .position(|&w| mnemonic(w).as_deref() == Some(unit))
    else {
        return out;
    };
    let word = v.words[at];
    let nj = u32::from(vscr_before(v, at) & NJ != 0);
    let (floor, operands) = float_floor(unit);
    for &(name, shift) in operands {
        for bits in words(start_vr(v, field(word, shift))) {
            out.insert(format!("nj:{nj}/{name}:{}", class(bits)));
        }
    }
    let scale = u32::from(field(word, 16));
    let b = words(start_vr(v, field(word, 11)));
    match floor {
        FloatFloor::Classes => {}
        FloatFloor::Compare => {
            // [AltiVec-PEM p:6-53 s:6.2] Word bit 21 is Rc.
            let rc = (word >> 10) & 1;
            out.insert(format!("rc:{rc}"));
            if rc == 1 {
                out.insert(cr6_feature(unit, v).into());
            }
        }
        FloatFloor::ToInteger { signed } => {
            out.insert(format!("uimm:{scale}"));
            for bits in b {
                out.insert(to_integer_class(bits, scale, signed).into());
            }
            out.insert(sat_feature(v, at));
        }
        FloatFloor::FromInteger { signed } => {
            out.insert(format!("uimm:{scale}"));
            for x in b {
                out.extend(from_integer_class(x, signed).into_iter().map(str::to_owned));
            }
            out.insert(sat_feature(v, at));
        }
    }
    out
}

/// The features the set of `unit` must show.
fn float_required(unit: &str) -> BTreeSet<String> {
    let (floor, operands) = float_floor(unit);
    let mut out: BTreeSet<String> = operands
        .iter()
        .flat_map(|&(name, _)| {
            (0..2).flat_map(move |nj| CLASSES.map(|c| format!("nj:{nj}/{name}:{c}")))
        })
        .collect();
    let scales = (0..32).map(|s| format!("uimm:{s}"));
    match floor {
        FloatFloor::Classes => {}
        FloatFloor::Compare => {
            out.extend(["rc:0".into(), "rc:1".into()]);
            out.extend(if unit == "vcmpbfp" {
                ["cr6:within", "cr6:outside"].map(str::to_owned).to_vec()
            } else {
                ["cr6:all", "cr6:none", "cr6:mixed"]
                    .map(str::to_owned)
                    .to_vec()
            });
        }
        FloatFloor::ToInteger { .. } => {
            out.extend(scales);
            out.extend(
                [
                    "lane:above",
                    "lane:below",
                    "lane:inside",
                    "lane:nan",
                    "sat:00",
                    "sat:01",
                    "sat:11",
                ]
                .map(str::to_owned),
            );
        }
        FloatFloor::FromInteger { signed } => {
            out.extend(scales);
            out.extend(["lane:zero", "lane:rounds", "sat:00", "sat:11"].map(str::to_owned));
            if signed {
                out.insert("lane:negative".into());
            }
        }
    }
    out
}

/// The features of `unit` its set lacks.
pub(super) fn float_missing(unit: &str, set: &PpuReferenceSet) -> Vec<String> {
    let shown: BTreeSet<String> = set
        .vectors
        .iter()
        .flat_map(|v| float_features(unit, v))
        .collect();
    float_required(unit).difference(&shown).cloned().collect()
}

/// The set written for vector floating-point `unit`, from the converter's
/// candidates.
pub(super) fn vmx_float_set(unit: &str, conversion: &Conversion) -> PpuReferenceSet {
    let candidates = conversion.candidates.get(unit).cloned().unwrap_or_default();
    let shown: Vec<BTreeSet<String>> = candidates.iter().map(|v| float_features(unit, v)).collect();
    PpuReferenceSet {
        schema_version: PPU_REFERENCE_SET_SCHEMA_VERSION,
        unit: PpuReferenceUnit::Instruction {
            mnemonic: unit.to_owned(),
        },
        vectors: select_by(&candidates, &shown),
    }
}
