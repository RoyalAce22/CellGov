//! The vector floor a converted unit's set must meet, and the features a
//! vector shows towards it.
//!
//! A feature names one thing a vector exercises:
//!
//! - a form of the word;
//! - an operand at one of its typed boundaries;
//! - an XER bit the word reads or writes;
//! - the values of the CR bits the word reads;
//! - whether the word's CR target is also a source;
//! - the field-mask count or SPR number the word names.
//!
//! A unit's floor is the set of features its set must show.

use std::collections::BTreeSet;

use crate::ppu_reference::{word_unit, PpuReferenceUnit, PpuReferenceVector, ReferenceField};

/// [PPC-Book1 p:32 s:3.2.2] XER bit 32 is SO, bit 33 OV, bit 34 CA.
const XER_SO: u64 = 0x8000_0000;
/// XER bit 33, OV.
const XER_OV: u64 = 0x4000_0000;
/// XER bit 34, CA.
const XER_CA: u64 = 0x2000_0000;

/// A word field that names a source register.
#[derive(Debug, Clone, Copy)]
pub(super) enum Field {
    /// Bits 6-10: RS, or RT as a source.
    Rs,
    /// Bits 11-15: RA.
    Ra,
    /// Bits 16-20: RB.
    Rb,
}

impl Field {
    fn of(self, word: u32) -> usize {
        let shift = match self {
            Self::Rs => 21,
            Self::Ra => 16,
            Self::Rb => 11,
        };
        ((word >> shift) & 31) as usize
    }
}

/// The immediate field a unit takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Imm {
    /// No immediate operand.
    None,
    /// A 16-bit signed immediate.
    Signed,
    /// A 16-bit unsigned immediate.
    Unsigned,
}

/// What a unit's floor asks of its set.
#[derive(Debug, Clone, Copy)]
pub(super) struct Floor {
    /// The source register fields; the set shows each at every operand boundary.
    pub(super) sources: &'static [Field],
    /// The immediate; the set shows it at every immediate boundary.
    pub(super) imm: Imm,
    /// Whether the word has an Rc bit; the set shows both forms, and Rc=1
    /// with SO set on entry.
    pub(super) rc: bool,
    /// Whether the word has an OE bit; the set shows both forms, and OE=1
    /// with SO set on entry and with OV set on exit.
    pub(super) oe: bool,
    /// Whether the unit reads CA; the set shows CA clear and CA set on entry.
    pub(super) ca_in: bool,
    /// Whether the unit writes CA; the set shows CA clear and CA set on exit.
    pub(super) ca_out: bool,
    /// Whether the unit reads `(RA|0)`; the set shows a word with RA = 0.
    pub(super) ra0: bool,
    /// Whether the unit has an L bit; the set shows both widths.
    pub(super) l: bool,
    /// Whether the unit combines two CR bits; the set shows each value of
    /// the two source bits, and a target bit equal to a source bit.
    pub(super) cr_logic: bool,
    /// Whether the unit copies one CR field to another; the set shows the
    /// two fields apart and the same.
    pub(super) mcrf: bool,
    /// The field mask counts the set shows.
    pub(super) fxm: Fxm,
    /// The special-purpose registers the set moves.
    pub(super) sprs: &'static [u16],
}

/// The field mask counts a CR move's set shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Fxm {
    /// The unit has no field mask.
    None,
    /// `mtcrf`: no field, one field and every field.
    Crf,
    /// `mtocrf` and `mfocrf`: one field and several fields.
    Ocrf,
}

const NONE: Floor = Floor {
    sources: &[],
    imm: Imm::None,
    rc: false,
    oe: false,
    ca_in: false,
    ca_out: false,
    ra0: false,
    l: false,
    cr_logic: false,
    mcrf: false,
    fxm: Fxm::None,
    sprs: &[],
};

const RA: &[Field] = &[Field::Ra];
const RA_RB: &[Field] = &[Field::Ra, Field::Rb];
const RS: &[Field] = &[Field::Rs];
const RS_RB: &[Field] = &[Field::Rs, Field::Rb];
const RS_RA: &[Field] = &[Field::Rs, Field::Ra];

/// An XO-form arithmetic unit with Rc and OE.
const fn xo(sources: &'static [Field], ca_in: bool, ca_out: bool) -> Floor {
    Floor {
        sources,
        rc: true,
        oe: true,
        ca_in,
        ca_out,
        ..NONE
    }
}

/// An X-form or rotate unit with Rc.
const fn x(sources: &'static [Field]) -> Floor {
    Floor {
        sources,
        rc: true,
        ..NONE
    }
}

/// A D-form unit.
const fn d(sources: &'static [Field], imm: Imm) -> Floor {
    Floor {
        sources,
        imm,
        ..NONE
    }
}

/// Each converted unit's floor.
///
/// [PPC-Book1 p:51 s:3.3.8] The arithmetic units with OE update SO and
/// OV; the carrying ones update CA, and the extended ones read it.
/// [PPC-Book1 p:60 s:3.3.9] The compares have an L bit.
/// [PPC-Book1 p:80 s:3.3.12.2] The algebraic shifts update CA.
///
/// # Panics
///
/// Panics if `unit` is not a converted unit.
pub(super) fn floor(unit: &str) -> Floor {
    match unit {
        "add" | "subf" | "divd" | "divdu" | "divw" | "divwu" | "mulld" | "mullw" => {
            xo(RA_RB, false, false)
        }
        "addc" | "subfc" => xo(RA_RB, false, true),
        "adde" | "subfe" => xo(RA_RB, true, true),
        "addme" | "addze" | "subfme" | "subfze" => xo(RA, true, true),
        "neg" => xo(RA, false, false),
        "mulhd" | "mulhdu" | "mulhw" | "mulhwu" => x(RA_RB),
        "addic" | "addic." | "subfic" => Floor {
            ca_out: true,
            ..d(RA, Imm::Signed)
        },
        "mulli" => d(RA, Imm::Signed),
        "addis" => Floor {
            ra0: true,
            ..d(RA, Imm::Signed)
        },
        "andi." | "andis." | "ori" | "oris" | "xori" | "xoris" => d(RS, Imm::Unsigned),
        "and" | "andc" | "eqv" | "nand" | "nor" | "or" | "orc" | "xor" | "sld" | "slw" | "srd"
        | "srw" | "rldcl" | "rldcr" | "rlwnm" => x(RS_RB),
        "srad" | "sraw" => Floor {
            ca_out: true,
            ..x(RS_RB)
        },
        "sradi" | "srawi" => Floor {
            ca_out: true,
            ..x(RS)
        },
        "cntlzd" | "cntlzw" | "extsb" | "extsh" | "extsw" | "rldic" | "rldicl" | "rldicr"
        | "rlwinm" => x(RS),
        "rldimi" | "rlwimi" => x(RS_RA),
        "cmp" | "cmpl" => Floor {
            l: true,
            ..d(RA_RB, Imm::None)
        },
        "cmpi" => Floor {
            l: true,
            ..d(RA, Imm::Signed)
        },
        "cmpli" => Floor {
            l: true,
            ..d(RA, Imm::Unsigned)
        },
        "td" | "tw" => d(RA_RB, Imm::None),
        "tdi" | "twi" => d(RA, Imm::Signed),
        "crand" | "crandc" | "creqv" | "crnand" | "crnor" | "cror" | "crorc" | "crxor" => Floor {
            cr_logic: true,
            ..NONE
        },
        "mcrf" => Floor { mcrf: true, ..NONE },
        "mfcr" => NONE,
        "mtcrf" => Floor {
            fxm: Fxm::Crf,
            ..NONE
        },
        "mtocrf" | "mfocrf" => Floor {
            fxm: Fxm::Ocrf,
            ..NONE
        },
        "mfspr" => Floor {
            sprs: &MFSPR_SPRS,
            ..NONE
        },
        "mtspr" => Floor {
            sprs: &MTSPR_SPRS,
            ..NONE
        },
        other => panic!("no floor for {other}"),
    }
}

/// The SPRs a problem-state program may read: XER, LR, CTR, VRSAVE, CTRL
/// and SPRG3.
const MFSPR_SPRS: [u16; 6] = [1, 8, 9, 256, 136, 259];

/// The SPRs a problem-state program may write and read back: XER, LR,
/// CTR and VRSAVE.
const MTSPR_SPRS: [u16; 4] = [1, 8, 9, 256];

/// The SPR number of an `mfspr` or `mtspr` word: its two 5-bit halves
/// swapped.
fn spr_number(word: u32) -> u16 {
    (((word >> 16) & 0x1f) | (((word >> 11) & 0x1f) << 5)) as u16
}

/// The CR bit `n` of `cr`, bit 0 the most significant.
fn cr_bit(cr: u32, n: u32) -> u32 {
    (cr >> (31 - n)) & 1
}

/// The count class of a CR move's field mask.
fn fxm_class(word: u32) -> &'static str {
    match ((word >> 12) & 0xff).count_ones() {
        0 => "none",
        1 => "one",
        8 => "all",
        _ => "several",
    }
}

/// The boundary class of a 64-bit operand.
fn operand_class(v: u64) -> &'static str {
    match v {
        0 => "zero",
        1 => "one",
        u64::MAX => "m1",
        0x7fff_ffff => "smax32",
        0x8000_0000 => "smin32",
        0x7fff_ffff_ffff_ffff => "smax64",
        0x8000_0000_0000_0000 => "smin64",
        _ if v >> 32 != 0 => "hi",
        _ => "other",
    }
}

/// The operand classes every source register must take.
const OPERAND_CLASSES: [&str; 8] = [
    "zero", "one", "m1", "smax32", "smin32", "smax64", "smin64", "hi",
];

/// The boundary class of a 16-bit immediate.
fn imm_class(imm: Imm, v: u16) -> &'static str {
    match (imm, v) {
        (_, 0) => "zero",
        (_, 1) => "one",
        (_, 0x7fff) => "max15",
        (_, 0x8000) => "bit15",
        (_, 0xffff) => "all",
        _ => "other",
    }
}

/// The immediate classes every immediate must take.
///
/// - A signed immediate: 0, 1, -1 and its two extremes.
/// - An unsigned immediate: 0, 1, its top bit alone, the bits below it,
///   and every bit.
const IMM_CLASSES: [&str; 5] = ["zero", "one", "max15", "bit15", "all"];

fn expected_xer(v: &PpuReferenceVector) -> Option<u64> {
    match v.expected.state.xer {
        ReferenceField::Value { value } => Some(value),
        _ => None,
    }
}

/// The features `v` shows towards `floor`, read from its first word of
/// `unit`.
pub(super) fn features(floor: &Floor, unit: &str, v: &PpuReferenceVector) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let of_unit = |w: &&u32| matches!(word_unit(**w), Some(PpuReferenceUnit::Instruction { mnemonic }) if mnemonic == unit);
    let Some(&word) = v.words.iter().find(of_unit) else {
        return out;
    };
    out.insert(format!("word:0x{word:08x}"));
    let gpr = |n: usize| {
        v.initial_state
            .gpr
            .get(&(n as u8))
            .copied()
            .unwrap_or_default()
    };
    for (i, field) in floor.sources.iter().enumerate() {
        let n = field.of(word);
        if floor.ra0 && n == 0 {
            continue;
        }
        out.insert(format!("op{i}:{}", operand_class(gpr(n))));
    }
    if floor.imm != Imm::None {
        out.insert(format!("imm:{}", imm_class(floor.imm, word as u16)));
    }
    let xer_in = v.initial_state.xer;
    let xer_out = expected_xer(v).unwrap_or_default();
    let rc = word & 1 == 1;
    let oe = (word >> 10) & 1 == 1;
    if floor.rc {
        out.insert(if rc { "rc" } else { "norc" }.into());
        if rc && xer_in & XER_SO != 0 {
            out.insert("rc-so-in".into());
        }
    }
    if floor.oe {
        out.insert(if oe { "oe" } else { "nooe" }.into());
        if oe && xer_in & XER_SO != 0 {
            out.insert("oe-so-in".into());
        }
        if oe && xer_out & XER_OV != 0 {
            out.insert("ov-out".into());
        }
    }
    if floor.ca_in {
        out.insert(format!("ca-in:{}", u8::from(xer_in & XER_CA != 0)));
    }
    if floor.ca_out {
        out.insert(format!("ca-out:{}", u8::from(xer_out & XER_CA != 0)));
    }
    if floor.ra0 && Field::Ra.of(word) == 0 {
        out.insert("ra0".into());
    }
    if floor.l {
        out.insert(format!("l{}", (word >> 21) & 1));
    }
    let cr = v.initial_state.cr;
    if floor.cr_logic {
        let (bt, ba, bb) = ((word >> 21) & 31, (word >> 16) & 31, (word >> 11) & 31);
        out.insert(format!("ab:{}{}", cr_bit(cr, ba), cr_bit(cr, bb)));
        if bt == ba || bt == bb {
            out.insert("bt-is-source".into());
        }
    }
    if floor.mcrf {
        let same = (word >> 23) & 7 == (word >> 18) & 7;
        out.insert(if same { "bf-eq-bfa" } else { "bf-ne-bfa" }.into());
    }
    if floor.fxm != Fxm::None {
        out.insert(format!("fxm:{}", fxm_class(word)));
    }
    if !floor.sprs.is_empty() {
        out.insert(format!("spr:{}", spr_number(word)));
    }
    out
}

/// The features a unit's set must show.
pub(super) fn required(floor: &Floor) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for i in 0..floor.sources.len() {
        for class in OPERAND_CLASSES {
            out.insert(format!("op{i}:{class}"));
        }
    }
    if floor.imm != Imm::None {
        for class in IMM_CLASSES {
            out.insert(format!("imm:{class}"));
        }
    }
    let mut add = |names: &[&str]| out.extend(names.iter().map(|n| (*n).to_owned()));
    if floor.rc {
        add(&["rc", "norc", "rc-so-in"]);
    }
    if floor.oe {
        add(&["oe", "nooe", "oe-so-in", "ov-out"]);
    }
    if floor.ca_in {
        add(&["ca-in:0", "ca-in:1"]);
    }
    if floor.ca_out {
        add(&["ca-out:0", "ca-out:1"]);
    }
    if floor.ra0 {
        add(&["ra0"]);
    }
    if floor.l {
        add(&["l0", "l1"]);
    }
    if floor.cr_logic {
        add(&["ab:00", "ab:01", "ab:10", "ab:11", "bt-is-source"]);
    }
    if floor.mcrf {
        add(&["bf-eq-bfa", "bf-ne-bfa"]);
    }
    match floor.fxm {
        Fxm::None => {}
        Fxm::Crf => add(&["fxm:none", "fxm:one", "fxm:all"]),
        Fxm::Ocrf => add(&["fxm:one", "fxm:several"]),
    }
    out.extend(floor.sprs.iter().map(|n| format!("spr:{n}")));
    out
}
