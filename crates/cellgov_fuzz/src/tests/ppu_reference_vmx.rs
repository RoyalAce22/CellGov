//! The vector units' sets, the floor of each set, and the `vscr` and
//! `vrsave` facility sets.
//!
//! The sets come from the table-probe captures that the converter reads.
//! A feature names one thing a vector exercises, read from the vector's
//! first word of the unit and the bit patterns of its start and expected
//! state. A unit's floor is the features its set must show.

use std::collections::BTreeSet;

use super::capture::{
    capture_source, facility_vectors, select_by, unobserved_stop, unsupported, value, Conversion,
    ENTRY_VSCR, MEMORY_BASE, NJ, SAT,
};
use crate::ppu_reference::{
    word_unit, PpuReferenceFacility, PpuReferenceInputState, PpuReferenceObservation,
    PpuReferenceProvenance, PpuReferenceSet, PpuReferenceState, PpuReferenceStateBase,
    PpuReferenceUnit, PpuReferenceVector, ReferenceField, PPU_REFERENCE_SET_SCHEMA_VERSION,
};

/// The vector units this module fills that move no memory:
///
/// - the shift controls;
/// - the data-stream instructions;
/// - the permutes, merges, splats and whole-register shifts;
/// - the logical instructions;
/// - the packs and unpacks;
/// - the VSCR moves.
pub(super) const VMX_UNITS: [&str; 49] = [
    "lvsl", "lvsr", "dss", "dssall", "dst", "dstst", "dststt", "dstt", "vperm", "vsel", "vsldoi",
    "vmrghb", "vmrghh", "vmrghw", "vmrglb", "vmrglh", "vmrglw", "vspltb", "vsplth", "vspltw",
    "vspltisb", "vspltish", "vspltisw", "vsl", "vsr", "vslo", "vsro", "vand", "vandc", "vnor",
    "vor", "vxor", "vpkpx", "vpkshss", "vpkshus", "vpkswss", "vpkswus", "vpkuhum", "vpkuhus",
    "vpkuwum", "vpkuwus", "vupkhpx", "vupkhsb", "vupkhsh", "vupklpx", "vupklsb", "vupklsh",
    "mfvscr", "mtvscr",
];

/// The vector loads and stores this module fills.
pub(super) const VMX_MEMORY_UNITS: [&str; 18] = [
    "lvebx", "lvehx", "lvewx", "lvlx", "lvlxl", "lvrx", "lvrxl", "lvx", "lvxl", "stvebx", "stvehx",
    "stvewx", "stvlx", "stvlxl", "stvrx", "stvrxl", "stvx", "stvxl",
];

/// A pack that saturates, and the range it saturates to.
#[derive(Debug, Clone, Copy)]
pub(super) struct Saturate {
    /// The source lane width in bytes.
    width: u32,
    signed: bool,
    lo: i64,
    hi: i64,
}

/// What a vector unit's floor asks of its set.
#[derive(Debug, Clone, Copy)]
pub(super) enum VmxFloor {
    /// Each byte offset of the effective address within a quadword.
    Address,
    /// `vperm` control bytes from vA only, from vB only, from both, and
    /// with the bits the permute ignores set.
    Permute,
    /// A `vsel` mask of zeros, of ones and of both.
    Select,
    /// Each `vsldoi` shift.
    Sldoi,
    /// Each `vsl` or `vsr` bit count, and vB bytes that disagree.
    BitShift,
    /// Each `vslo` or `vsro` octet count.
    OctetShift,
    /// Each splat index below the element count.
    Splat(u32),
    /// The splat immediate's edges.
    SplatImmediate,
    /// A saturating pack: each lane class, and SAT before and after.
    Pack(Saturate),
    /// A pack that never saturates: SAT clear and SAT set, each unchanged.
    PackModulo,
    /// A signed unpack: negative and positive source lanes.
    Unpack { high: bool, width: u32 },
    /// A pixel unpack: each pixel's top bit clear and set.
    UnpackPixel { high: bool },
    /// Each data stream.
    Stream,
    /// `mtvscr`: NJ and SAT each clear and set, a reserved bit, and the
    /// vB words it ignores.
    Mtvscr,
    /// `mfvscr`: NJ and SAT each clear and set.
    Mfvscr,
    /// No feature: the set needs one vector.
    Plain,
}

const fn pack(width: u32, signed: bool, lo: i64, hi: i64) -> VmxFloor {
    VmxFloor::Pack(Saturate {
        width,
        signed,
        lo,
        hi,
    })
}

/// Each vector unit's floor.
///
/// [AltiVec-PEM p:6-9 s:6.2] `dss` stops a data stream, and `dssall`
/// every stream.
/// [AltiVec-PEM p:6-10 s:6.2] `dst` starts the data stream STRM names.
/// [AltiVec-PEM p:6-14 s:6.2] An element load reads the element at EA.
/// [AltiVec-PEM p:6-21 s:6.2] `lvsl` reads the low four bits of EA.
/// [CBE-Handbook p:710 s:24.9] The left and right loads and stores move
/// an unaligned quadword in two parts, the left part and the right part.
/// [CBE-Handbook p:744 s:A.3.3] A right form at a quadword-aligned EA
/// accesses no storage.
/// [AltiVec-PEM p:6-112 s:6.2] `vperm` selects each byte from vA or vB by
/// bit 3 of its control byte, and reads only bits 3:7.
/// [AltiVec-PEM p:6-133 s:6.2] `vsel` takes each bit from vB where the
/// mask bit is 1.
/// [AltiVec-PEM p:6-134 s:6.2] `vsl` shifts by vB bits 125:127, and vD is
/// undefined unless every vB byte holds the same count.
/// [AltiVec-PEM p:6-136 s:6.2] `vsldoi` shifts by SH octets.
/// [AltiVec-PEM p:6-138 s:6.2] `vslo` shifts by vB bits 121:124 octets.
/// [AltiVec-PEM p:6-140 s:6.2] A splat copies the element UIMM names.
/// [AltiVec-PEM p:6-142 s:6.2] A splat immediate sign-extends SIMM.
/// [AltiVec-PEM p:6-115 s:6.2] `vpkshss` saturates each signed half word
/// to a signed byte and sets SAT when one saturates.
/// [AltiVec-PEM p:6-120 s:6.2] `vpkuhus` saturates each unsigned half word
/// to an unsigned byte.
/// [AltiVec-PEM p:6-119 s:6.2] `vpkuhum` keeps each half word's low byte.
/// [AltiVec-PEM p:6-171 s:6.2] `vupkhpx` sign-extends each pixel's bit 0.
/// [AltiVec-PEM p:6-172 s:6.2] `vupkhsb` sign-extends each byte.
/// [AltiVec-PEM p:6-23 s:6.2] `mfvscr` reads the VSCR.
/// [AltiVec-PEM p:6-24 s:6.2] `mtvscr` writes the VSCR from vB bits
/// 96:127.
///
/// # Panics
///
/// Panics if `unit` is not one of [`VMX_UNITS`] or [`VMX_MEMORY_UNITS`].
pub(super) fn vmx_floor(unit: &str) -> VmxFloor {
    use VmxFloor as F;
    match unit {
        u if VMX_MEMORY_UNITS.contains(&u) => F::Address,
        "lvsl" | "lvsr" => F::Address,
        "dss" | "dst" | "dstst" | "dststt" | "dstt" => F::Stream,
        "vperm" => F::Permute,
        "vsel" => F::Select,
        "vsldoi" => F::Sldoi,
        "vsl" | "vsr" => F::BitShift,
        "vslo" | "vsro" => F::OctetShift,
        "vspltb" => F::Splat(16),
        "vsplth" => F::Splat(8),
        "vspltw" => F::Splat(4),
        "vspltisb" | "vspltish" | "vspltisw" => F::SplatImmediate,
        "vpkuhus" => pack(2, false, 0, 0xff),
        "vpkuwus" => pack(4, false, 0, 0xffff),
        "vpkshus" => pack(2, true, 0, 0xff),
        "vpkswus" => pack(4, true, 0, 0xffff),
        "vpkshss" => pack(2, true, -0x80, 0x7f),
        "vpkswss" => pack(4, true, -0x8000, 0x7fff),
        "vpkuhum" | "vpkuwum" | "vpkpx" => F::PackModulo,
        "vupkhsb" => F::Unpack {
            high: true,
            width: 1,
        },
        "vupklsb" => F::Unpack {
            high: false,
            width: 1,
        },
        "vupkhsh" => F::Unpack {
            high: true,
            width: 2,
        },
        "vupklsh" => F::Unpack {
            high: false,
            width: 2,
        },
        "vupkhpx" => F::UnpackPixel { high: true },
        "vupklpx" => F::UnpackPixel { high: false },
        "mtvscr" => F::Mtvscr,
        "mfvscr" => F::Mfvscr,
        "dssall" | "vmrghb" | "vmrghh" | "vmrghw" | "vmrglb" | "vmrglh" | "vmrglw" | "vand"
        | "vandc" | "vnor" | "vor" | "vxor" => F::Plain,
        other => panic!("no vector floor for {other}"),
    }
}

/// A word field: the five bits at `shift`.
pub(super) fn field(word: u32, shift: u32) -> u8 {
    ((word >> shift) & 31) as u8
}

/// The start value of VR `n`, or zero when the vector loads none.
pub(super) fn start_vr(v: &PpuReferenceVector, n: u8) -> u128 {
    v.initial_state.vr_hex.get(&n).map_or(0, |h| {
        u128::from_str_radix(h, 16).unwrap_or_else(|e| panic!("{}: v{n} {h:?}: {e}", v.name))
    })
}

/// The VSCR `v` expects.
///
/// # Panics
///
/// Panics if the vector does not expect a VSCR value.
fn expected_vscr(v: &PpuReferenceVector) -> u32 {
    match v.expected.state.vscr {
        ReferenceField::Value { value } => value,
        _ => panic!("{}: the expected VSCR is not a value", v.name),
    }
}

/// The VSCR before word `at` of `v` runs: the one the last `mtvscr` before
/// it loaded, or the start VSCR.
///
/// [AltiVec-PEM p:6-24 s:6.2] `mtvscr` loads the VSCR from vB bits 96:127.
pub(super) fn vscr_before(v: &PpuReferenceVector, at: usize) -> u32 {
    let loaded = v.words[..at]
        .iter()
        .rposition(|&w| matches!(mnemonic(w).as_deref(), Some("mtvscr")));
    match loaded {
        Some(m) => vr_before(v, m, field(v.words[m], 11)) as u32,
        None => v.initial_state.vscr.unwrap_or(ENTRY_VSCR),
    }
}

/// VR `n` before word `at` of `v` runs: its start value, or the splat the
/// last `vspltisw` into it wrote.
///
/// [AltiVec-PEM p:6-144 s:6.2] `vspltisw` sign-extends SIMM into each word
/// of vD.
///
/// # Panics
///
/// Panics if another vector word or a vector load before `at` writes VR `n`.
fn vr_before(v: &PpuReferenceVector, at: usize, n: u8) -> u128 {
    let mut vr = start_vr(v, n);
    for &w in &v.words[..at] {
        let m = mnemonic(w);
        let vector = w >> 26 == 4 || m.as_deref().is_some_and(|name| name.starts_with("lv"));
        if !vector || field(w, 21) != n || m.as_deref() == Some("mtvscr") {
            continue;
        }
        assert_eq!(
            m.as_deref(),
            Some("vspltisw"),
            "{}: word {w:#010x} writes v{n} before an mtvscr reads it",
            v.name
        );
        let simm = i32::from(field(w, 16)) - if field(w, 16) & 0x10 != 0 { 32 } else { 0 };
        vr = u128::from(simm as u32) * 0x0000_0001_0000_0001_0000_0001_0000_0001;
    }
    vr
}

fn sat_before(v: &PpuReferenceVector, at: usize) -> u32 {
    vscr_before(v, at) & SAT
}

pub(super) fn mnemonic(word: u32) -> Option<String> {
    match word_unit(word)? {
        PpuReferenceUnit::Instruction { mnemonic } => Some(mnemonic),
        _ => None,
    }
}

/// The lanes of `v`, each `width` bytes, high lane first, as signed or
/// unsigned values.
pub(super) fn lanes(v: u128, width: u32, signed: bool) -> Vec<i64> {
    let bits = 8 * width;
    (0..128 / bits)
        .rev()
        .map(|k| {
            let raw = (v >> (k * bits)) as u64 & ((1u64 << bits) - 1);
            if signed && raw >> (bits - 1) == 1 {
                raw as i64 - (1i64 << bits)
            } else {
                raw as i64
            }
        })
        .collect()
}

/// The features `v` shows towards `floor`, read from its first word of
/// `unit`.
pub(super) fn vmx_features(
    floor: VmxFloor,
    unit: &str,
    v: &PpuReferenceVector,
) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Some(at) = v
        .words
        .iter()
        .position(|&w| mnemonic(w).as_deref() == Some(unit))
    else {
        return out;
    };
    let word = v.words[at];
    let (ra, rb) = (field(word, 16), field(word, 11));
    let (va, vb, vc) = (
        start_vr(v, ra),
        start_vr(v, rb),
        start_vr(v, field(word, 6)),
    );
    let bytes = |x: u128| x.to_be_bytes();
    match floor {
        VmxFloor::Address => {
            let gpr = |n: u8| v.initial_state.gpr.get(&n).copied().unwrap_or_default();
            let base = if ra == 0 { 0 } else { gpr(ra) };
            out.insert(format!("ea:{}", base.wrapping_add(gpr(rb)) & 15));
        }
        VmxFloor::Permute => {
            let ctl = bytes(vc);
            let from_b = ctl.iter().filter(|&&c| c & 0x10 != 0).count();
            out.insert(
                match from_b {
                    0 => "ctl:a",
                    16 => "ctl:b",
                    _ => "ctl:both",
                }
                .into(),
            );
            if ctl.iter().any(|&c| c & 0xe0 != 0) {
                out.insert("ctl:ignored-bits".into());
            }
        }
        VmxFloor::Select => {
            out.insert(
                match vc {
                    0 => "mask:zeros",
                    u128::MAX => "mask:ones",
                    _ => "mask:both",
                }
                .into(),
            );
        }
        VmxFloor::Sldoi => {
            out.insert(format!("sh:{}", (word >> 6) & 15));
        }
        VmxFloor::BitShift => {
            let counts: BTreeSet<u8> = bytes(vb).iter().map(|b| b & 7).collect();
            match counts.iter().next() {
                Some(c) if counts.len() == 1 => out.insert(format!("sh:{c}")),
                _ => out.insert("uneven".into()),
            };
        }
        VmxFloor::OctetShift => {
            out.insert(format!("oct:{}", (bytes(vb)[15] >> 3) & 15));
        }
        VmxFloor::Splat(n) => {
            if u32::from(ra) < n {
                out.insert(format!("uimm:{ra}"));
            }
        }
        VmxFloor::SplatImmediate => {
            let simm = i32::from(ra) - if ra & 0x10 != 0 { 32 } else { 0 };
            out.insert(format!("simm:{simm}"));
        }
        VmxFloor::Pack(s) => {
            for lane in lanes(va, s.width, s.signed)
                .into_iter()
                .chain(lanes(vb, s.width, s.signed))
            {
                out.insert(lane_class(s, lane).into());
            }
            out.insert(sat_feature(v, at));
        }
        VmxFloor::PackModulo => {
            out.insert(sat_feature(v, at));
        }
        VmxFloor::Unpack { high, width } => {
            let half = if high { vb >> 64 } else { vb } as u64;
            let n = 64 / (8 * width);
            for k in 0..n {
                let top = half >> (8 * width * (k + 1) - 1) & 1;
                out.insert(if top == 1 { "lane:neg" } else { "lane:pos" }.into());
            }
        }
        VmxFloor::UnpackPixel { high } => {
            let half = if high { vb >> 64 } else { vb } as u64;
            for k in 0..4 {
                let top = half >> (16 * k + 15) & 1;
                out.insert(if top == 1 { "pixel:set" } else { "pixel:clear" }.into());
            }
        }
        VmxFloor::Stream => {
            out.insert(format!("strm:{}", (word >> 21) & 3));
        }
        VmxFloor::Mtvscr => {
            let low = vb as u32;
            out.insert(format!("nj:{}", u32::from(low & NJ != 0)));
            out.insert(format!("sat:{}", low & SAT));
            if low & !(NJ | SAT) != 0 {
                out.insert("reserved".into());
            }
            if vb >> 32 != 0 {
                out.insert("ignored-words".into());
            }
        }
        VmxFloor::Mfvscr => {
            let vscr = expected_vscr(v);
            out.insert(format!("nj:{}", u32::from(vscr & NJ != 0)));
            out.insert(format!("sat:{}", vscr & SAT));
        }
        VmxFloor::Plain => {}
    }
    out
}

/// The class of a source lane against a saturating pack's range.
fn lane_class(s: Saturate, lane: i64) -> &'static str {
    match lane {
        l if l > s.hi => "lane:above",
        l if l < s.lo => "lane:below",
        l if l == s.hi => "lane:max",
        l if l == s.lo => "lane:min",
        _ => "lane:inside",
    }
}

/// SAT before the word at `at` and after the vector, as `sat:<before><after>`.
pub(super) fn sat_feature(v: &PpuReferenceVector, at: usize) -> String {
    format!("sat:{}{}", sat_before(v, at), expected_vscr(v) & SAT)
}

/// The features a vector unit's set must show.
pub(super) fn vmx_required(floor: VmxFloor) -> BTreeSet<String> {
    let range = |name: &str, n: u32| (0..n).map(|k| format!("{name}:{k}")).collect::<Vec<_>>();
    let words = |w: &[&str]| w.iter().map(|&s| s.to_owned()).collect::<Vec<_>>();
    match floor {
        VmxFloor::Address => range("ea", 16),
        VmxFloor::Permute => words(&["ctl:a", "ctl:b", "ctl:both", "ctl:ignored-bits"]),
        VmxFloor::Select => words(&["mask:zeros", "mask:ones", "mask:both"]),
        VmxFloor::Sldoi => range("sh", 16),
        VmxFloor::BitShift => {
            let mut out = range("sh", 8);
            out.push("uneven".into());
            out
        }
        VmxFloor::OctetShift => range("oct", 16),
        VmxFloor::Splat(n) => range("uimm", n),
        VmxFloor::SplatImmediate => words(&["simm:-16", "simm:-1", "simm:0", "simm:15"]),
        VmxFloor::Pack(s) => {
            let mut out = words(&[
                "lane:above",
                "lane:max",
                "lane:min",
                "lane:inside",
                "sat:00",
                "sat:01",
                "sat:11",
            ]);
            if s.signed && s.lo > -(1i64 << (8 * s.width - 1)) {
                out.push("lane:below".into());
            }
            out
        }
        VmxFloor::PackModulo => words(&["sat:00", "sat:11"]),
        VmxFloor::Unpack { .. } => words(&["lane:neg", "lane:pos"]),
        VmxFloor::UnpackPixel { .. } => words(&["pixel:set", "pixel:clear"]),
        VmxFloor::Stream => range("strm", 4),
        VmxFloor::Mtvscr => words(&[
            "nj:0",
            "nj:1",
            "sat:0",
            "sat:1",
            "reserved",
            "ignored-words",
        ]),
        VmxFloor::Mfvscr => words(&["nj:0", "nj:1", "sat:0", "sat:1"]),
        VmxFloor::Plain => Vec::new(),
    }
    .into_iter()
    .collect()
}

/// The set written for vector `unit`, from the converter's candidates.
pub(super) fn vmx_set(unit: &str, conversion: &Conversion) -> PpuReferenceSet {
    let candidates = conversion.candidates.get(unit).cloned().unwrap_or_default();
    let floor = vmx_floor(unit);
    let shown: Vec<BTreeSet<String>> = candidates
        .iter()
        .map(|v| vmx_features(floor, unit, v))
        .collect();
    PpuReferenceSet {
        schema_version: PPU_REFERENCE_SET_SCHEMA_VERSION,
        unit: PpuReferenceUnit::Instruction {
            mnemonic: unit.to_owned(),
        },
        vectors: select_by(&candidates, &shown),
    }
}

/// The `vscr` facility's set: every case of the `ppu_vscr_rules` capture,
/// each over several words that load the VSCR and read it back.
pub(super) fn vscr_facility_set() -> PpuReferenceSet {
    PpuReferenceSet {
        schema_version: PPU_REFERENCE_SET_SCHEMA_VERSION,
        unit: PpuReferenceUnit::Facility {
            name: PpuReferenceFacility::Vscr,
        },
        vectors: facility_vectors("ppu_vscr_rules"),
    }
}

const MFVRSAVE_R3: u32 = 0x7c60_42a6;
const MTVRSAVE_R4: u32 = 0x7c80_43a6;
const NOP: u32 = 0x6000_0000;

/// The `vrsave` facility's set, from the `ppu_vrsave_entry` capture:
///
/// - the VRSAVE the main thread and a child thread start with, each held
///   through a no-op;
/// - a write of a value with its high word set, read back.
///
/// # Panics
///
/// Panics if the capture is missing or reports a failed step.
pub(super) fn vrsave_facility_set() -> PpuReferenceSet {
    const PROBE: &str = "ppu_vrsave_entry";
    let path = cellgov_testkit::console_cases::probe_dir(PROBE)
        .join("ps3")
        .join(cellgov_testkit::console_cases::PROFILE)
        .join("cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &frame[8..];
    assert_eq!(p.len(), 28, "{PROBE}: payload size");
    let word = |k: usize| u32::from_be_bytes(p[4 * k..4 * k + 4].try_into().expect("4 bytes"));
    assert_eq!(word(0), 0, "{PROBE}: step {} failed", word(0));
    // The child writes word 5 before it exits with 0x5AFE; any other
    // exit value means word 5 can be the probe's sentinel.
    assert_eq!(word(6), 0x5afe, "{PROBE}: the child's exit value");
    let source = capture_source(PROBE);
    let provenance = PpuReferenceProvenance::HardwareCapture {
        capture_id: format!("{PROBE}/{}", cellgov_testkit::console_cases::PROFILE),
        device: source.device,
        environment: source.environment,
        source_sha256: source.frame_sha256,
    };
    let written = 0xffff_ffff_c000_0001;
    let read_back = u64::from(word(4)) << 32 | u64::from(word(3));
    let vectors = vec![
        vrsave_vector(
            "vrsave-entry-main",
            &provenance,
            &[NOP],
            &[],
            word(2),
            word(2),
            None,
        ),
        vrsave_vector(
            "vrsave-entry-child",
            &provenance,
            &[NOP],
            &[],
            word(5),
            word(5),
            None,
        ),
        vrsave_vector(
            "vrsave-write-read",
            &provenance,
            &[MTVRSAVE_R4, MFVRSAVE_R3],
            &[(4, written)],
            0,
            word(3),
            Some(read_back),
        ),
    ];
    PpuReferenceSet {
        schema_version: PPU_REFERENCE_SET_SCHEMA_VERSION,
        unit: PpuReferenceUnit::Facility {
            name: PpuReferenceFacility::Vrsave,
        },
        vectors,
    }
}

/// A vector that runs `words` from a zeroed state with `gprs` and `vrsave`
/// loaded.
///
/// It expects VRSAVE `end`, and r3 `r3` when the caller gives one.
fn vrsave_vector(
    name: &str,
    provenance: &PpuReferenceProvenance,
    words: &[u32],
    gprs: &[(u8, u64)],
    vrsave: u32,
    end: u32,
    r3: Option<u64>,
) -> PpuReferenceVector {
    const RUNTIME: &str = "the console capture does not observe the runtime's effect pipeline";
    let mut gpr = vec![0u64; 32];
    for &(n, v) in gprs {
        gpr[usize::from(n)] = v;
    }
    if let Some(r3) = r3 {
        gpr[3] = r3;
    }
    let n = words.len() as u64;
    PpuReferenceVector {
        name: name.to_owned(),
        provenance: provenance.clone(),
        words: words.to_vec(),
        initial_state: PpuReferenceInputState {
            base: PpuReferenceStateBase::Zeroed,
            gpr: gprs.iter().copied().collect(),
            fpr: Default::default(),
            vr_hex: Default::default(),
            pc: 0,
            cr: 0,
            lr: 0,
            ctr: 0,
            xer: 0,
            fpscr: Some(0),
            vrsave,
            vscr: Some(ENTRY_VSCR),
            tb: 0,
            reservation: None,
        },
        memory_base: MEMORY_BASE,
        initial_memory: vec![0; 16],
        expected: PpuReferenceObservation {
            state: PpuReferenceState {
                gpr: value(gpr),
                fpr: value(vec![0; 32]),
                vr_hex: value(vec![format!("{:032x}", 0); 32]),
                pc: value(4 * n),
                cr: value(0),
                lr: value(0),
                ctr: value(0),
                xer: value(0),
                fpscr: value(0),
                vrsave: value(end),
                vscr: value(ENTRY_VSCR),
                tb: ReferenceField::Undefined {
                    reason: "the time base advances while the words run".to_owned(),
                },
                reservation: value(None),
            },
            memory: value(vec![0; 16]),
            stop: unobserved_stop(),
            retired: value(n),
            staged_effects: unsupported(RUNTIME),
            committed_effects: unsupported(RUNTIME),
            reservations: unsupported(RUNTIME),
            store_buffer: unsupported(RUNTIME),
            commit_error: unsupported(RUNTIME),
            fault_discarded: unsupported(RUNTIME),
        },
    }
}
