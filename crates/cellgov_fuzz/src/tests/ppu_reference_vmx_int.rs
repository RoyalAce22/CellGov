//! The vector integer units' sets and the floor of each set: the adds and
//! subtracts, averages, maximums and minimums, compares, rotates and
//! shifts, multiplies and sums across.
//!
//! A feature names one thing a vector shows. The module reads each feature
//! from these parts of the vector:
//!
//! - the first word of the vector that decodes as the unit;
//! - the start VRs;
//! - the VSCR before and after that word;
//! - the expected CR.
//!
//! The floor computes each lane's exact result from the start VRs, and the
//! lane's class comes from that result. Every expected value comes from the
//! console capture.

use std::collections::BTreeSet;

use super::capture::{select_by, Conversion};
use super::vmx::{field, lanes, mnemonic, sat_feature, start_vr};
use crate::ppu_reference::{
    PpuReferenceSet, PpuReferenceUnit, PpuReferenceVector, ReferenceField,
    PPU_REFERENCE_SET_SCHEMA_VERSION,
};

/// The vector integer units this module fills.
pub(super) const VMX_INT_UNITS: [&str; 80] = [
    "vaddcuw",
    "vaddsbs",
    "vaddshs",
    "vaddubm",
    "vaddubs",
    "vadduhm",
    "vadduhs",
    "vadduwm",
    "vadduws",
    "vsubcuw",
    "vsubsbs",
    "vsubshs",
    "vsubsws",
    "vsububm",
    "vsububs",
    "vsubuhm",
    "vsubuhs",
    "vsubuwm",
    "vsubuws",
    "vavgsb",
    "vavgsh",
    "vavgsw",
    "vavgub",
    "vavguh",
    "vavguw",
    "vmaxsb",
    "vmaxsh",
    "vmaxsw",
    "vmaxub",
    "vmaxuh",
    "vmaxuw",
    "vminsb",
    "vminsh",
    "vminsw",
    "vminub",
    "vminuh",
    "vminuw",
    "vcmpequb",
    "vcmpequh",
    "vcmpequw",
    "vcmpgtsb",
    "vcmpgtsh",
    "vcmpgtsw",
    "vcmpgtub",
    "vcmpgtuh",
    "vcmpgtuw",
    "vrlb",
    "vrlh",
    "vrlw",
    "vslb",
    "vslh",
    "vslw",
    "vsrb",
    "vsrh",
    "vsrw",
    "vsrab",
    "vsrah",
    "vsraw",
    "vmulesb",
    "vmulesh",
    "vmuleub",
    "vmuleuh",
    "vmulosb",
    "vmulosh",
    "vmuloub",
    "vmulouh",
    "vmhaddshs",
    "vmhraddshs",
    "vmladduhm",
    "vmsummbm",
    "vmsumshm",
    "vmsumshs",
    "vmsumubm",
    "vmsumuhm",
    "vmsumuhs",
    "vsum2sws",
    "vsum4sbs",
    "vsum4shs",
    "vsum4ubs",
    "vsumsws",
];

/// A lane: its width in bytes and whether it holds a signed value.
#[derive(Debug, Clone, Copy)]
struct Shape {
    width: u32,
    signed: bool,
}

impl Shape {
    /// The shape a unit's sign and width letters name: `s` or `u`, then
    /// `b`, `h` or `w`.
    fn of(sign: u8, width: u8) -> Self {
        Self {
            width: match width {
                b'b' => 1,
                b'h' => 2,
                _ => 4,
            },
            signed: sign == b's',
        }
    }

    fn bits(self) -> u32 {
        8 * self.width
    }

    /// The least and the greatest value the lane holds.
    fn range(self) -> (i64, i64) {
        let bits = self.bits();
        if self.signed {
            (-(1i64 << (bits - 1)), (1i64 << (bits - 1)) - 1)
        } else {
            (0, (1i64 << bits) - 1)
        }
    }

    fn lanes(self, v: u128) -> Vec<i64> {
        lanes(v, self.width, self.signed)
    }
}

const U8: Shape = Shape {
    width: 1,
    signed: false,
};
const S8: Shape = Shape {
    width: 1,
    signed: true,
};
const U16: Shape = Shape {
    width: 2,
    signed: false,
};
const S16: Shape = Shape {
    width: 2,
    signed: true,
};
const U32: Shape = Shape {
    width: 4,
    signed: false,
};
const S32: Shape = Shape {
    width: 4,
    signed: true,
};

/// What a vector integer unit's floor asks of its set.
#[derive(Debug, Clone, Copy)]
enum IntFloor {
    /// A modulo result: a lane whose exact result fits and one whose
    /// result wraps, with SAT clear and set before the word.
    Modulo,
    /// A saturating result: each class of exact lane result the unit can
    /// reach, and SAT before and after.
    Saturate { above: bool, below: bool },
    /// A maximum or minimum: lanes whose sign bits differ and equal lanes,
    /// with SAT clear and set before the word.
    MaxMin(Shape),
    /// A compare: the plain form, and the record form with CR6 for all
    /// true, all false and mixed, over lanes whose sign bits differ and
    /// equal lanes.
    Compare(Shape),
    /// A rotate or shift: a count of 0, a count of the lane width less
    /// one, and a count lane with bits above its count field.
    Shift { bits: u32, arithmetic: bool },
    /// An even or odd multiply: both operands at the lane's extreme, and
    /// for a signed one, a negative product; SAT clear and set before.
    Multiply { shape: Shape, odd: bool },
}

/// Each vector integer unit's floor.
///
/// [AltiVec-PEM p:6-35 s:6.2] `vaddubm` keeps the low byte of each sum.
/// [AltiVec-PEM p:6-30 s:6.2] `vaddcuw` gives the carry out of each word
/// sum.
/// [AltiVec-PEM p:6-36 s:6.2] `vaddubs` saturates each unsigned sum and
/// sets SAT when one saturates.
/// [AltiVec-PEM p:6-32 s:6.2] `vaddsbs` saturates each signed sum.
/// [AltiVec-PEM p:6-161 s:6.2] `vsububs` saturates each unsigned
/// difference.
/// [AltiVec-PEM p:6-46 s:6.2] `vavgub` gives each rounded mean from a sum
/// one bit wider than the lane.
/// [AltiVec-PEM p:6-73 s:6.2] `vmaxsb` compares each pair as signed values.
/// [AltiVec-PEM p:6-56 s:6.2] A recording compare sets CR6 to all true,
/// a zero, all false and a zero.
/// [AltiVec-PEM p:6-129 s:6.2] `vrlb` rotates each byte by the low three
/// bits of its vB byte.
/// [AltiVec-PEM p:6-148 s:6.2] `vsrab` shifts the sign bit in.
/// [AltiVec-PEM p:6-101 s:6.2] `vmulesb` multiplies each even pair of
/// signed bytes.
/// [AltiVec-PEM p:6-79 s:6.2] `vmhaddshs` adds the high bits of each
/// product to vC and saturates.
/// [AltiVec-PEM p:6-97 s:6.2] `vmsumshs` adds each word's products to vC
/// and saturates.
/// [AltiVec-PEM p:6-170 s:6.2] `vsum4ubs` adds each word's bytes to vB and
/// saturates.
///
/// # Panics
///
/// Panics if `unit` is not one of [`VMX_INT_UNITS`].
fn int_floor(unit: &str) -> IntFloor {
    let u = unit.as_bytes();
    let saturate = |signed: bool, sub: bool| IntFloor::Saturate {
        above: signed || !sub,
        below: signed || sub,
    };
    match unit {
        "vaddcuw" | "vsubcuw" | "vmladduhm" | "vmsummbm" | "vmsumshm" | "vmsumubm" | "vmsumuhm" => {
            IntFloor::Modulo
        }
        "vmhaddshs" | "vmhraddshs" | "vmsumshs" | "vsum2sws" | "vsum4sbs" | "vsum4shs"
        | "vsumsws" => saturate(true, false),
        "vmsumuhs" | "vsum4ubs" => saturate(false, false),
        _ if unit.starts_with("vadd") || unit.starts_with("vsub") => match u[6] {
            b'm' => IntFloor::Modulo,
            _ => saturate(u[4] == b's', unit.starts_with("vsub")),
        },
        _ if unit.starts_with("vavg") => IntFloor::Modulo,
        _ if unit.starts_with("vmax") || unit.starts_with("vmin") => {
            IntFloor::MaxMin(Shape::of(u[4], u[5]))
        }
        _ if unit.starts_with("vcmp") => IntFloor::Compare(Shape::of(u[6], u[7])),
        _ if unit.starts_with("vmul") => IntFloor::Multiply {
            shape: Shape::of(u[5], u[6]),
            odd: u[4] == b'o',
        },
        "vrlb" | "vrlh" | "vrlw" | "vslb" | "vslh" | "vslw" | "vsrb" | "vsrh" | "vsrw"
        | "vsrab" | "vsrah" | "vsraw" => IntFloor::Shift {
            bits: Shape::of(b'u', u[u.len() - 1]).bits(),
            arithmetic: unit.starts_with("vsra"),
        },
        other => panic!("no vector integer floor for {other}"),
    }
}

/// The shape of a unit's result lane and the exact result of each lane.
///
/// The exact result is the value before the unit wraps or saturates it.
/// Returns `None` for a unit whose floor reads no exact result.
fn exact(unit: &str, a: u128, b: u128, c: u128) -> Option<(Shape, Vec<i64>)> {
    let u = unit.as_bytes();
    let pairwise = |s: Shape, f: fn(i64, i64) -> i64| -> Vec<i64> {
        s.lanes(a)
            .into_iter()
            .zip(s.lanes(b))
            .map(|(x, y)| f(x, y))
            .collect()
    };
    let add: fn(i64, i64) -> i64 = |x, y| x + y;
    let sub: fn(i64, i64) -> i64 = |x, y| x - y;
    // The word lanes of `acc`, each plus the sum of the products of
    // vA's and vB's lanes inside that word.
    let multiply_sum = |sa: Shape, sb: Shape, acc: Shape, plus: u128| -> Vec<i64> {
        let per = (4 / sa.width) as usize;
        let products: Vec<i64> = sa
            .lanes(a)
            .into_iter()
            .zip(sb.lanes(b))
            .map(|(x, y)| x * y)
            .collect();
        acc.lanes(plus)
            .into_iter()
            .zip(products.chunks(per))
            .map(|(z, p)| z + p.iter().sum::<i64>())
            .collect()
    };
    // The word lanes of vB, each plus the sum of vA's lanes inside that
    // word.
    let sum_across = |sa: Shape, acc: Shape| -> Vec<i64> {
        let per = (4 / sa.width) as usize;
        acc.lanes(b)
            .into_iter()
            .zip(sa.lanes(a).chunks(per))
            .map(|(z, x)| z + x.iter().sum::<i64>())
            .collect()
    };
    let triple = |f: fn(i64, i64, i64) -> i64| -> Vec<i64> {
        let (x, y, z) = (S16.lanes(a), S16.lanes(b), S16.lanes(c));
        (0..8).map(|k| f(x[k], y[k], z[k])).collect()
    };
    Some(match unit {
        "vaddcuw" => (U32, pairwise(U32, add)),
        "vsubcuw" => (U32, pairwise(U32, sub)),
        _ if unit.starts_with("vadd") || unit.starts_with("vsub") => {
            let s = Shape::of(u[4], u[5]);
            let f = if unit.starts_with("vadd") { add } else { sub };
            (s, pairwise(s, f))
        }
        _ if unit.starts_with("vavg") => {
            let s = Shape::of(u[4], u[5]);
            (s, pairwise(s, |x, y| x + y + 1))
        }
        "vmladduhm" => {
            let (x, y, z) = (U16.lanes(a), U16.lanes(b), U16.lanes(c));
            (U16, (0..8).map(|k| x[k] * y[k] + z[k]).collect())
        }
        "vmhaddshs" => (S16, triple(|x, y, z| ((x * y) >> 15) + z)),
        "vmhraddshs" => (S16, triple(|x, y, z| ((x * y + 0x4000) >> 15) + z)),
        "vmsumubm" => (U32, multiply_sum(U8, U8, U32, c)),
        "vmsummbm" => (S32, multiply_sum(S8, U8, S32, c)),
        "vmsumuhm" | "vmsumuhs" => (U32, multiply_sum(U16, U16, U32, c)),
        "vmsumshm" | "vmsumshs" => (S32, multiply_sum(S16, S16, S32, c)),
        "vsum4ubs" => (U32, sum_across(U8, U32)),
        "vsum4sbs" => (S32, sum_across(S8, S32)),
        "vsum4shs" => (S32, sum_across(S16, S32)),
        "vsum2sws" => {
            let (x, y) = (S32.lanes(a), S32.lanes(b));
            (S32, vec![x[0] + x[1] + y[1], x[2] + x[3] + y[3]])
        }
        "vsumsws" => (
            S32,
            vec![S32.lanes(a).iter().sum::<i64>() + S32.lanes(b)[3]],
        ),
        _ => return None,
    })
}

/// The class of an exact lane result against the lane's range. A modulo
/// floor's classes are `fits` and `wraps`; a saturating floor's are
/// `above`, `below`, `max`, `min` and `inside`.
fn lane_class(floor: IntFloor, s: Shape, x: i64) -> &'static str {
    let (lo, hi) = s.range();
    match floor {
        IntFloor::Modulo if (lo..=hi).contains(&x) => "lane:fits",
        IntFloor::Modulo => "lane:wraps",
        _ if x > hi => "lane:above",
        _ if x < lo => "lane:below",
        _ if x == hi => "lane:max",
        _ if x == lo => "lane:min",
        _ => "lane:inside",
    }
}

/// Adds to `out` the features of each lane pair of `a` and `b`.
///
/// - `lane:sign-differs`: the sign bits of the pair differ.
/// - `lane:equal`: the two lanes are equal.
fn pair_features(s: Shape, a: u128, b: u128, out: &mut BTreeSet<String>) {
    let top = |x: i64| (x >> (s.bits() - 1)) & 1;
    let raw = Shape { signed: false, ..s };
    for (x, y) in raw.lanes(a).into_iter().zip(raw.lanes(b)) {
        if top(x) != top(y) {
            out.insert("lane:sign-differs".into());
        }
        if x == y {
            out.insert("lane:equal".into());
        }
    }
}

/// CR field 6 of `cr`.
fn cr6(cr: u32) -> u32 {
    (cr >> 4) & 0xf
}

/// The features `v` shows towards the floor of `unit`, read from the first
/// word of `v` that decodes as `unit`.
fn int_features(unit: &str, v: &PpuReferenceVector) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Some(at) = v
        .words
        .iter()
        .position(|&w| mnemonic(w).as_deref() == Some(unit))
    else {
        return out;
    };
    let word = v.words[at];
    let (a, b, c) = (
        start_vr(v, field(word, 16)),
        start_vr(v, field(word, 11)),
        start_vr(v, field(word, 6)),
    );
    let floor = int_floor(unit);
    match floor {
        IntFloor::Modulo | IntFloor::Saturate { .. } => {
            let (s, results) = exact(unit, a, b, c).expect("an exact result");
            for x in results {
                out.insert(lane_class(floor, s, x).into());
            }
            out.insert(sat_feature(v, at));
        }
        IntFloor::MaxMin(s) => {
            pair_features(s, a, b, &mut out);
            out.insert(sat_feature(v, at));
        }
        IntFloor::Compare(s) => {
            pair_features(s, a, b, &mut out);
            // [AltiVec-PEM p:6-54 s:6.2] Word bit 21 is Rc.
            let feature = if word & 0x400 == 0 {
                "rc:0"
            } else {
                let ReferenceField::Value { value: cr } = v.expected.state.cr else {
                    panic!("{}: the expected CR is not a value", v.name);
                };
                // [AltiVec-PEM p:6-54 s:6.2] A recording compare writes
                // CR6 as all-true, a zero, none-true and a zero, so no
                // other value is a compare result.
                match cr6(cr) {
                    0x8 => "cr6:all",
                    0x2 => "cr6:none",
                    0x0 => "cr6:mixed",
                    other => panic!("{}: CR6 {other:#x} is no compare result", v.name),
                }
            };
            out.insert(feature.into());
        }
        IntFloor::Shift { bits, arithmetic } => {
            let lane = Shape {
                width: bits / 8,
                signed: false,
            };
            for (x, n) in lane.lanes(a).into_iter().zip(lane.lanes(b)) {
                let count = n & i64::from(bits - 1);
                if count == 0 {
                    out.insert("count:0".into());
                }
                if count == i64::from(bits - 1) {
                    out.insert("count:max".into());
                }
                if n >> bits.trailing_zeros() != 0 {
                    out.insert("count:high-bits".into());
                }
                if arithmetic && count != 0 && x >> (bits - 1) == 1 {
                    out.insert("lane:negative".into());
                }
            }
        }
        IntFloor::Multiply { shape, odd } => {
            let (lo, hi) = shape.range();
            let extreme = if shape.signed { lo } else { hi };
            let pairs = shape.lanes(a).into_iter().zip(shape.lanes(b));
            for (x, y) in pairs.skip(usize::from(odd)).step_by(2) {
                if x == extreme && y == extreme {
                    out.insert("operand:extreme".into());
                }
                if x * y < 0 {
                    out.insert("product:negative".into());
                }
            }
            out.insert(sat_feature(v, at));
        }
    }
    out
}

/// The features the set of `unit` must show.
fn int_required(unit: &str) -> BTreeSet<String> {
    let mut out: Vec<&str> = match int_floor(unit) {
        IntFloor::Modulo => vec!["lane:fits", "lane:wraps", "sat:00", "sat:11"],
        IntFloor::Saturate { above, below } => {
            let mut out = vec![
                "lane:max",
                "lane:min",
                "lane:inside",
                "sat:00",
                "sat:01",
                "sat:11",
            ];
            if above {
                out.push("lane:above");
            }
            if below {
                out.push("lane:below");
            }
            out
        }
        IntFloor::MaxMin(_) => vec!["lane:sign-differs", "lane:equal", "sat:00", "sat:11"],
        IntFloor::Compare(_) => vec![
            "lane:sign-differs",
            "lane:equal",
            "rc:0",
            "cr6:all",
            "cr6:none",
            "cr6:mixed",
        ],
        IntFloor::Shift { arithmetic, .. } => {
            let mut out = vec!["count:0", "count:max", "count:high-bits"];
            if arithmetic {
                out.push("lane:negative");
            }
            out
        }
        IntFloor::Multiply { shape, .. } => {
            let mut out = vec!["operand:extreme", "sat:00", "sat:11"];
            if shape.signed {
                out.push("product:negative");
            }
            out
        }
    };
    out.sort_unstable();
    out.into_iter().map(str::to_owned).collect()
}

/// The features of `unit` its set lacks.
pub(super) fn int_missing(unit: &str, set: &PpuReferenceSet) -> Vec<String> {
    let shown: BTreeSet<String> = set
        .vectors
        .iter()
        .flat_map(|v| int_features(unit, v))
        .collect();
    int_required(unit).difference(&shown).cloned().collect()
}

/// The set written for vector integer `unit`, from the converter's
/// candidates.
pub(super) fn vmx_int_set(unit: &str, conversion: &Conversion) -> PpuReferenceSet {
    let candidates = conversion.candidates.get(unit).cloned().unwrap_or_default();
    let shown: Vec<BTreeSet<String>> = candidates.iter().map(|v| int_features(unit, v)).collect();
    PpuReferenceSet {
        schema_version: PPU_REFERENCE_SET_SCHEMA_VERSION,
        unit: PpuReferenceUnit::Instruction {
            mnemonic: unit.to_owned(),
        },
        vectors: select_by(&candidates, &shown),
    }
}
