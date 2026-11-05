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

use super::fp::{class, single_class, CLASSES};
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
    /// The storage access the unit makes, if any.
    pub(super) access: Option<Access>,
}

/// The storage access of a load or store unit.
#[derive(Debug, Clone, Copy)]
pub(super) struct Access {
    /// The bytes the access moves.
    pub(super) size: u64,
    /// How the word forms the address.
    pub(super) form: AddressForm,
    /// Whether the unit writes the address back to RA.
    pub(super) update: bool,
    pub(super) store: bool,
    /// Whether the unit reverses the bytes it moves.
    pub(super) reverse: bool,
    pub(super) data: Data,
}

/// The register and format a load or store moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Data {
    /// A GPR's bytes.
    Int,
    /// A single-format value, widened into or narrowed from an FPR.
    Single,
    /// A double-format value in an FPR.
    Double,
    /// The low word of an FPR, as `stfiwx` stores it.
    FprWord,
}

/// How a load or store word forms its effective address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AddressForm {
    /// `(RA|0)` plus a 16-bit displacement, or a DS displacement.
    Displacement,
    /// `(RA|0)` plus RB.
    Index,
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
    access: None,
};

/// The floor of an integer load or store unit.
///
/// A non-update indexed form reads `(RA|0)`, so its set shows RA = 0.
const fn access(size: u64, form: AddressForm, update: bool, store: bool, reverse: bool) -> Floor {
    moves(size, form, update, store, reverse, Data::Int)
}

/// The floor of a load or store unit that moves `data`.
const fn moves(
    size: u64,
    form: AddressForm,
    update: bool,
    store: bool,
    reverse: bool,
    data: Data,
) -> Floor {
    Floor {
        ra0: matches!(form, AddressForm::Index) && !update,
        access: Some(Access {
            size,
            form,
            update,
            store,
            reverse,
            data,
        }),
        ..NONE
    }
}

/// The floor of a floating-point load or store unit.
const fn float(data: Data, form: AddressForm, update: bool, store: bool) -> Floor {
    let size = match data {
        Data::Double => 8,
        _ => 4,
    };
    moves(size, form, update, store, false, data)
}

const fn load(size: u64, form: AddressForm, update: bool) -> Floor {
    access(size, form, update, false, false)
}

const fn store(size: u64, form: AddressForm, update: bool) -> Floor {
    access(size, form, update, true, false)
}

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
        other => memory_floor(other).unwrap_or_else(|| panic!("no floor for {other}")),
    }
}

/// The floor of a load or store unit, or `None` for any other unit.
fn memory_floor(unit: &str) -> Option<Floor> {
    use AddressForm::{Displacement as D, Index as X};
    Some(match unit {
        "lbz" => load(1, D, false),
        "lbzu" => load(1, D, true),
        "lbzx" => load(1, X, false),
        "lbzux" => load(1, X, true),
        "lhz" | "lha" => load(2, D, false),
        "lhzu" | "lhau" => load(2, D, true),
        "lhzx" | "lhax" => load(2, X, false),
        "lhzux" | "lhaux" => load(2, X, true),
        "lwz" | "lwa" => load(4, D, false),
        "lwzu" => load(4, D, true),
        "lwzx" | "lwax" => load(4, X, false),
        "lwzux" | "lwaux" => load(4, X, true),
        "ld" => load(8, D, false),
        "ldu" => load(8, D, true),
        "ldx" => load(8, X, false),
        "ldux" => load(8, X, true),
        "lhbrx" => access(2, X, false, false, true),
        "lwbrx" => access(4, X, false, false, true),
        "ldbrx" => access(8, X, false, false, true),
        "stb" => store(1, D, false),
        "stbu" => store(1, D, true),
        "stbx" => store(1, X, false),
        "stbux" => store(1, X, true),
        "sth" => store(2, D, false),
        "sthu" => store(2, D, true),
        "sthx" => store(2, X, false),
        "sthux" => store(2, X, true),
        "stw" => store(4, D, false),
        "stwu" => store(4, D, true),
        "stwx" => store(4, X, false),
        "stwux" => store(4, X, true),
        "std" => store(8, D, false),
        "stdu" => store(8, D, true),
        "stdx" => store(8, X, false),
        "stdux" => store(8, X, true),
        "sthbrx" => access(2, X, false, true, true),
        "stwbrx" => access(4, X, false, true, true),
        "sdbrx" => access(8, X, false, true, true),
        other => return float_floor(other),
    })
}

/// The floor of a floating-point load or store unit, or `None` for any
/// other unit.
///
/// [PPC-Book1 p:104 s:4.6.2] `lfs` widens a single-format word into an
/// FPR.
/// [PPC-Book1 p:105 s:4.6.2] `lfd` loads a doubleword unchanged.
/// [PPC-Book1 p:107 s:4.6.3] `stfs` narrows an FPR to a single-format
/// word.
/// [PPC-Book1 p:108 s:4.6.3] `stfd` stores the doubleword unchanged.
/// [PPC-Book1 p:109 s:4.6.3] `stfiwx` stores the FPR's low word.
fn float_floor(unit: &str) -> Option<Floor> {
    use AddressForm::{Displacement as D, Index as X};
    use Data::{Double, FprWord, Single};
    Some(match unit {
        "lfs" => float(Single, D, false, false),
        "lfsu" => float(Single, D, true, false),
        "lfsx" => float(Single, X, false, false),
        "lfsux" => float(Single, X, true, false),
        "lfd" => float(Double, D, false, false),
        "lfdu" => float(Double, D, true, false),
        "lfdx" => float(Double, X, false, false),
        "lfdux" => float(Double, X, true, false),
        "stfs" => float(Single, D, false, true),
        "stfsu" => float(Single, D, true, true),
        "stfsx" => float(Single, X, false, true),
        "stfsux" => float(Single, X, true, true),
        "stfd" => float(Double, D, false, true),
        "stfdu" => float(Double, D, true, true),
        "stfdx" => float(Double, X, false, true),
        "stfdux" => float(Double, X, true, true),
        "stfiwx" => float(FprWord, X, false, true),
        _ => return None,
    })
}

/// A sign-extended 16-bit value.
fn exts16(v: u32) -> u64 {
    i64::from(v as u16 as i16) as u64
}

/// The GPR bank the unit's word at `at` reads: the start values, with each
/// `addi` before it applied. The words before a load or store only place
/// its address.
fn bank_at(v: &PpuReferenceVector, at: usize) -> [u64; 32] {
    let mut g = [0u64; 32];
    for (&n, &value) in &v.initial_state.gpr {
        g[usize::from(n)] = value;
    }
    for &w in &v.words[..at] {
        if w >> 26 == 14 {
            let ra = Field::Ra.of(w);
            let base = if ra == 0 { 0 } else { g[ra] };
            g[Field::Rs.of(w)] = base.wrapping_add(exts16(w));
        }
    }
    g
}

/// The features a load or store word shows: its address form's operand
/// sign, the alignment of its address, and the datum's top bit or the
/// stored value's class. A floating-point move shows the class of the
/// datum it loads or the FPR it stores.
fn access_features(a: &Access, v: &PpuReferenceVector, at: usize, out: &mut BTreeSet<String>) {
    let word = v.words[at];
    let g = bank_at(v, at);
    let ra = Field::Ra.of(word);
    let base = if ra == 0 { 0 } else { g[ra] };
    let sign = |x: u64| match x as i64 {
        0 => "zero",
        x if x > 0 => "pos",
        _ => "neg",
    };
    let offset = match a.form {
        AddressForm::Displacement => {
            // A DS-form word keeps its two low bits for the opcode extension.
            let ds = matches!(word >> 26, 58 | 62);
            let d = exts16(if ds { word & 0xfffc } else { word });
            out.insert(format!("disp:{}", sign(d)));
            d
        }
        AddressForm::Index => {
            let index = g[Field::Rb.of(word)];
            if ra != 0 {
                out.insert(format!("index:{}", sign(index)));
            }
            index
        }
    };
    let ea = base.wrapping_add(offset).wrapping_sub(v.memory_base) as usize;
    if a.size > 1 {
        let aligned = (ea as u64).is_multiple_of(a.size);
        out.insert(
            if aligned {
                "ea:aligned"
            } else {
                "ea:misaligned"
            }
            .into(),
        );
    }
    let frs = || {
        v.initial_state
            .fpr
            .get(&(Field::Rs.of(word) as u8))
            .copied()
            .unwrap_or_default()
    };
    match (a.data, a.store) {
        (Data::Int, true) => {
            out.insert(format!("rs:{}", operand_class(g[Field::Rs.of(word)])));
        }
        (Data::FprWord, true) => {
            out.insert(format!("frs:{}", operand_class(frs())));
        }
        (data, true) => {
            out.insert(format!("frs:{}", class(frs(), data == Data::Single)));
        }
        (Data::Int, false) => {
            let first = if a.reverse {
                ea + a.size as usize - 1
            } else {
                ea
            };
            if let Some(byte) = v.initial_memory.get(first) {
                out.insert(format!("msb:{}", byte >> 7));
            }
        }
        (data, false) => {
            let Some(bytes) = v.initial_memory.get(ea..ea + a.size as usize) else {
                return;
            };
            let name = match data {
                Data::Single => single_class(u32::from_be_bytes(
                    bytes.try_into().expect("a single is 4 bytes"),
                )),
                _ => class(
                    u64::from_be_bytes(bytes.try_into().expect("a double is 8 bytes")),
                    false,
                ),
            };
            out.insert(format!("data:{name}"));
        }
    }
}

/// The features a load or store unit's set must show.
fn access_required(a: &Access, out: &mut BTreeSet<String>) {
    let mut add = |names: &[&str]| out.extend(names.iter().map(|n| (*n).to_owned()));
    match (a.form, a.update) {
        (AddressForm::Displacement, _) => add(&["disp:zero", "disp:pos", "disp:neg"]),
        (AddressForm::Index, false) => add(&["index:zero", "index:pos"]),
        (AddressForm::Index, true) => add(&["index:zero", "index:pos", "index:neg"]),
    }
    if a.size > 1 {
        add(&["ea:aligned", "ea:misaligned"]);
    }
    match (a.data, a.store) {
        (Data::Int, true) => add(&["rs:zero", "rs:m1", "rs:hi"]),
        (Data::Int, false) => add(&["msb:0", "msb:1"]),
        (Data::FprWord, _) => add(&["frs:zero", "frs:m1", "frs:hi"]),
        (_, true) => out.extend(CLASSES.map(|c| format!("frs:{c}"))),
        (_, false) => out.extend(CLASSES.map(|c| format!("data:{c}"))),
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
    let Some(at) = v.words.iter().position(|w| of_unit(&w)) else {
        return out;
    };
    let word = v.words[at];
    if let Some(a) = &floor.access {
        access_features(a, v, at, &mut out);
    }
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
    if let Some(a) = &floor.access {
        access_required(a, &mut out);
    }
    out
}
