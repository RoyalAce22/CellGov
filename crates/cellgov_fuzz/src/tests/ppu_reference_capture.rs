//! The converter from a table-probe console capture to PPU reference
//! vector sets: one set per unit, each vector one captured case.
//!
//! A vector starts from the state the case loads. Its expected end takes
//! each recorded slot, CR, XER and FPSCR from the capture, and every
//! other GPR at its start value: the case's words write only the
//! registers the case records. CellGov runs each case for two checks:
//!
//! - its recorded result reads no register the routine keeps;
//! - its words write no GPR the case leaves unrecorded.
//!
//! The converter skips a case that fails either check. No expected
//! value comes from CellGov.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use cellgov_ppu::differential::{execute_state_in, PpuStateSnapshot};
use cellgov_testkit::console_cases::{capture, parse_table, Capture, Case, Reg, Run, PROFILE};

use super::floor::{features, floor};
use crate::ppu_reference::{
    word_unit, PpuReferenceInputState, PpuReferenceObservation, PpuReferenceProvenance,
    PpuReferenceSet, PpuReferenceState, PpuReferenceStateBase, PpuReferenceStop, PpuReferenceUnit,
    PpuReferenceVector, ReferenceField, MAX_PPU_REFERENCE_VECTORS,
    PPU_REFERENCE_SET_SCHEMA_VERSION,
};

/// The address of the data bytes every reference vector names.
const MEMORY_BASE: u64 = 0x1000_0000;

/// The count of data bytes each vector names. A fixed-point case reads and
/// writes none, so they end as they start.
const DATA_BYTES: usize = 16;

/// The VSCR a new PPU thread starts with: NJ set.
const ENTRY_VSCR: u32 = 0x0001_0000;

/// The GPRs the probe routine keeps for itself: r0 is its scratch, r11
/// and r12 its data and result pointers.
const ROUTINE_GPRS: [usize; 3] = [0, 11, 12];

/// The units this converter fills.
pub(super) const UNITS: [&str; 70] = [
    "add", "addc", "adde", "addic", "addic.", "addis", "addme", "addze", "and", "andc", "andi.",
    "andis.", "cmp", "cmpi", "cmpl", "cmpli", "cntlzd", "cntlzw", "divd", "divdu", "divw", "divwu",
    "eqv", "extsb", "extsh", "extsw", "mulhd", "mulhdu", "mulhw", "mulhwu", "mulld", "mulli",
    "mullw", "nand", "neg", "nor", "or", "orc", "ori", "oris", "rldcl", "rldcr", "rldic", "rldicl",
    "rldicr", "rldimi", "rlwimi", "rlwinm", "rlwnm", "sld", "slw", "srad", "sradi", "sraw",
    "srawi", "srd", "srw", "subf", "subfc", "subfe", "subfic", "subfme", "subfze", "td", "tdi",
    "tw", "twi", "xor", "xori", "xoris",
];

/// Why the converter left a captured case out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Skip {
    /// The case's unit is not one this converter fills.
    OtherUnit,
    /// The case names a memory buffer.
    Memory,
    /// The two console runs disagree.
    Varies,
    /// CellGov refuses one of the words.
    Refused,
    /// The result depends on a register the routine keeps for itself.
    ReadsRoutineRegister,
    /// The words write a GPR the case does not record.
    WritesUnrecorded,
    /// The console run ended the process, so the capture did not complete.
    Ended,
}

/// What the converter made of every committed capture.
#[derive(Debug, Default)]
pub(super) struct Conversion {
    /// Each unit's candidate vectors, in probe and case order.
    pub(super) candidates: BTreeMap<String, Vec<PpuReferenceVector>>,
    /// How many cases each reason left out.
    pub(super) skipped: BTreeMap<Skip, usize>,
}

/// `tests/micro/` of the repository.
fn micro_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/micro")
}

/// Every PPU probe with a case table and a committed capture under the
/// reference profile, in name order.
pub(super) fn captured_probes() -> Vec<String> {
    let mut probes: Vec<String> = std::fs::read_dir(micro_dir())
        .expect("tests/micro reads")
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|dir| {
            dir.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("ppu_"))
                && dir.join("cases.tsv").is_file()
                && dir
                    .join("ps3")
                    .join(PROFILE)
                    .join("cgov_frame.bin")
                    .is_file()
        })
        .map(|dir| {
            dir.file_name()
                .expect("a directory name")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    probes.sort();
    probes
}

/// The console fields of a capture's `provenance.json`.
struct CaptureSource {
    device: String,
    environment: String,
    frame_sha256: String,
}

fn capture_source(probe: &str) -> CaptureSource {
    let path = micro_dir()
        .join(probe)
        .join("ps3")
        .join(PROFILE)
        .join("provenance.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let field = |v: &serde_json::Value, name: &str| {
        v.as_str()
            .unwrap_or_else(|| panic!("{}: no {name}", path.display()))
            .to_owned()
    };
    CaptureSource {
        device: field(&p["console"]["model"], "console.model"),
        environment: format!(
            "firmware {}, {}, PPU thread",
            field(&p["console"]["firmware"], "console.firmware"),
            field(&p["console"]["cfw"], "console.cfw"),
        ),
        frame_sha256: field(&p["frame"]["sha256"], "frame.sha256"),
    }
}

/// The unit a case covers: the unit of its last word.
fn case_unit(case: &Case) -> Option<String> {
    match word_unit(*case.words.last()?)? {
        PpuReferenceUnit::Instruction { mnemonic } if UNITS.contains(&mnemonic.as_str()) => {
            Some(mnemonic)
        }
        _ => None,
    }
}

/// CellGov's state after the case's words from `start`.
fn cellgov_end(case: &Case, start: &PpuStateSnapshot) -> Option<PpuStateSnapshot> {
    let mut s = start.clone();
    for &w in &case.words {
        s = execute_state_in(w, &s, 0, Vec::new()).ok()?.0;
    }
    Some(s)
}

/// The recorded components of `s`: each recorded GPR, then CR and XER.
fn recorded(case: &Case, s: &PpuStateSnapshot) -> Vec<u64> {
    let mut out: Vec<u64> = case
        .record
        .iter()
        .filter_map(|r| match *r {
            Reg::Gpr(n) => Some(s.gpr[usize::from(n)]),
            _ => None,
        })
        .collect();
    out.extend([u64::from(s.cr), s.xer]);
    out
}

/// `start` with every routine register set to a value no case loads.
fn perturbed(start: &PpuStateSnapshot) -> PpuStateSnapshot {
    let mut s = start.clone();
    for n in ROUTINE_GPRS {
        s.gpr[n] = 0xa5a5_0000_0000_0000 | n as u64;
    }
    s
}

/// The expected GPR bank: the start values, with each recorded GPR the
/// console's.
fn expected_gprs(case: &Case, start: &PpuStateSnapshot, run: &Run) -> Vec<u64> {
    let mut gpr = start.gpr.to_vec();
    for (r, slot) in case.slots() {
        if let Reg::Gpr(n) = r {
            gpr[usize::from(n)] = run.slots[slot];
        }
    }
    gpr
}

/// Whether CellGov's end leaves every unrecorded GPR at its start value.
fn writes_only_recorded(case: &Case, start: &PpuStateSnapshot, end: &PpuStateSnapshot) -> bool {
    (0..start.gpr.len())
        .all(|n| start.gpr[n] == end.gpr[n] || case.record.contains(&Reg::Gpr(n as u8)))
}

/// Why `case` is left out, or `None` when it converts.
fn skip_reason(case: &Case, start: &PpuStateSnapshot, runs: &[Run; 2]) -> Option<Skip> {
    if case_unit(case).is_none() {
        return Some(Skip::OtherUnit);
    }
    if case.mem.is_some() {
        return Some(Skip::Memory);
    }
    if runs[0] != runs[1] {
        return Some(Skip::Varies);
    }
    let Some(end) = cellgov_end(case, start) else {
        return Some(Skip::Refused);
    };
    // A refusal only under other routine registers is a dependence on them.
    let Some(moved) = cellgov_end(case, &perturbed(start)) else {
        return Some(Skip::ReadsRoutineRegister);
    };
    if recorded(case, &end) != recorded(case, &moved) {
        return Some(Skip::ReadsRoutineRegister);
    }
    if !writes_only_recorded(case, start, &end) {
        return Some(Skip::WritesUnrecorded);
    }
    None
}

fn value<T>(value: T) -> ReferenceField<T> {
    ReferenceField::Value { value }
}

fn unsupported<T>(reason: &str) -> ReferenceField<T> {
    ReferenceField::Unsupported {
        reason: reason.to_owned(),
    }
}

/// The vector's start: the state the case loads.
fn input_state(case: &Case, start: &PpuStateSnapshot) -> PpuReferenceInputState {
    let nonzero = |bank: &[u64]| {
        bank.iter()
            .enumerate()
            .filter(|(_, v)| **v != 0)
            .map(|(n, v)| (n as u8, *v))
            .collect()
    };
    PpuReferenceInputState {
        base: PpuReferenceStateBase::Zeroed,
        gpr: nonzero(&start.gpr),
        fpr: nonzero(&start.fpr),
        vr_hex: start
            .vr
            .iter()
            .enumerate()
            .filter(|(_, v)| **v != 0)
            .map(|(n, v)| (n as u8, format!("{v:032x}")))
            .collect(),
        pc: 0,
        cr: case.cr,
        lr: 0,
        ctr: 0,
        xer: case.xer,
        fpscr: Some(case.fpscr),
        vrsave: 0,
        vscr: Some(ENTRY_VSCR),
        tb: 0,
        reservation: None,
    }
}

/// The runtime fields a console capture does not observe.
fn unobserved_stop() -> PpuReferenceStop {
    const STOP: &str = "the console capture does not observe the runtime's stop attribution";
    PpuReferenceStop {
        reason: unsupported(STOP),
        fault: unsupported(STOP),
        pc: unsupported(STOP),
        lr: unsupported(STOP),
        syscall_lev: unsupported(STOP),
        faulting_ea: unsupported(STOP),
        fault_registers: unsupported(STOP),
        syscall_args: unsupported(STOP),
    }
}

/// The expected end of a case the console ran as `run`.
fn expected(case: &Case, start: &PpuStateSnapshot, run: &Run) -> PpuReferenceObservation {
    const RUNTIME: &str = "the console capture does not observe the runtime's effect pipeline";
    let words = case.words.len() as u64;
    PpuReferenceObservation {
        state: PpuReferenceState {
            gpr: value(expected_gprs(case, start, run)),
            fpr: unsupported("the fixed-point case records no floating-point register"),
            vr_hex: unsupported("the fixed-point case records no vector register"),
            pc: value(4 * words),
            cr: value(run.cr),
            lr: value(0),
            ctr: value(0),
            xer: value(run.xer),
            fpscr: value(run.fpscr),
            vrsave: value(0),
            vscr: value(ENTRY_VSCR),
            tb: ReferenceField::Undefined {
                reason: "the time base advances while the case runs".to_owned(),
            },
            reservation: value(None),
        },
        memory: value(vec![0; DATA_BYTES]),
        stop: unobserved_stop(),
        retired: value(words),
        staged_effects: unsupported(RUNTIME),
        committed_effects: unsupported(RUNTIME),
        reservations: unsupported(RUNTIME),
        store_buffer: unsupported(RUNTIME),
        commit_error: unsupported(RUNTIME),
        fault_discarded: unsupported(RUNTIME),
    }
}

fn vector(
    probe: &str,
    source: &CaptureSource,
    case: &Case,
    start: &PpuStateSnapshot,
    run: &Run,
) -> PpuReferenceVector {
    PpuReferenceVector {
        name: format!("{probe}-{}", case.id),
        provenance: PpuReferenceProvenance::HardwareCapture {
            capture_id: format!("{probe}/{PROFILE}"),
            device: source.device.clone(),
            environment: source.environment.clone(),
            source_sha256: source.frame_sha256.clone(),
        },
        words: case.words.clone(),
        initial_state: input_state(case, start),
        memory_base: MEMORY_BASE,
        initial_memory: vec![0; DATA_BYTES],
        expected: expected(case, start, run),
    }
}

/// Every probe's cases, as candidate vectors per unit.
pub(super) fn convert(probes: &[String]) -> Conversion {
    let mut out = Conversion::default();
    for probe in probes {
        convert_probe(probe, &mut out);
    }
    out
}

fn convert_probe(probe: &str, out: &mut Conversion) {
    let path = micro_dir().join(probe).join("cases.tsv");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let cases = parse_table(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let runs = match capture(probe, cases.len()) {
        Capture::Done(runs) => runs,
        Capture::Ended { .. } => {
            for case in &cases {
                let skip = if case_unit(case).is_some() {
                    Skip::Ended
                } else {
                    Skip::OtherUnit
                };
                *out.skipped.entry(skip).or_default() += 1;
            }
            return;
        }
    };
    let source = capture_source(probe);
    for (index, (case, runs)) in cases.iter().zip(&runs).enumerate() {
        let start = case.initial(index, 0);
        if let Some(skip) = skip_reason(case, &start, runs) {
            *out.skipped.entry(skip).or_default() += 1;
            continue;
        }
        let unit = case_unit(case).expect("a converted case has a unit");
        out.candidates
            .entry(unit)
            .or_default()
            .push(vector(probe, &source, case, &start, &runs[0]));
    }
}

/// Up to [`MAX_PPU_REFERENCE_VECTORS`] of `candidates`, in candidate order.
///
/// The selection takes two passes:
///
/// 1. The cover: each pick is the candidate that shows the most floor
///    features not yet shown, until no candidate adds one.
/// 2. The fill: `spread` order over the rest, up to the cap.
fn select(unit: &str, candidates: &[PpuReferenceVector]) -> Vec<PpuReferenceVector> {
    let floor = floor(unit);
    let shown: Vec<BTreeSet<String>> = candidates.iter().map(|v| features(&floor, v)).collect();
    let mut taken = vec![false; candidates.len()];
    let mut covered: BTreeSet<&String> = BTreeSet::new();
    let mut count = 0;
    while count < MAX_PPU_REFERENCE_VECTORS {
        let gain = |i: usize| shown[i].iter().filter(|f| !covered.contains(f)).count();
        let Some(best) = (0..candidates.len())
            .filter(|&i| !taken[i] && gain(i) > 0)
            .max_by_key(|&i| (gain(i), std::cmp::Reverse(i)))
        else {
            break;
        };
        taken[best] = true;
        covered.extend(&shown[best]);
        count += 1;
    }
    let rest: Vec<usize> = (0..candidates.len()).filter(|&i| !taken[i]).collect();
    let fill = MAX_PPU_REFERENCE_VECTORS - count;
    for i in spread(&rest.iter().collect::<Vec<_>>())
        .into_iter()
        .take(fill)
    {
        taken[*i] = true;
    }
    candidates
        .iter()
        .zip(&taken)
        .filter(|(_, t)| **t)
        .map(|(v, _)| v.clone())
        .collect()
}

/// `items` reordered so each prefix spreads across the whole: the first,
/// the last, then the midpoints of the gaps left, widest first.
fn spread<'a, T>(items: &[&'a T]) -> Vec<&'a T> {
    let n = items.len();
    if n <= 2 {
        return items.to_vec();
    }
    let mut taken = vec![false; n];
    let mut order = vec![0, n - 1];
    taken[0] = true;
    taken[n - 1] = true;
    let mut gaps = vec![(0, n - 1)];
    while !gaps.is_empty() {
        let mut next = Vec::new();
        for (lo, hi) in gaps {
            if hi - lo < 2 {
                continue;
            }
            let mid = lo + (hi - lo) / 2;
            if !taken[mid] {
                taken[mid] = true;
                order.push(mid);
            }
            next.push((lo, mid));
            next.push((mid, hi));
        }
        gaps = next;
    }
    order.into_iter().map(|i| items[i]).collect()
}

/// The set the converter writes for `unit`.
pub(super) fn unit_set(unit: &str, candidates: &[PpuReferenceVector]) -> PpuReferenceSet {
    PpuReferenceSet {
        schema_version: PPU_REFERENCE_SET_SCHEMA_VERSION,
        unit: PpuReferenceUnit::Instruction {
            mnemonic: unit.to_owned(),
        },
        vectors: select(unit, candidates),
    }
}

/// The text of a converted file: indented JSON in field order.
///
/// The text puts each of these on one line:
///
/// - an array of plain values;
/// - an object of at most two plain fields.
///
/// The text ends with a newline.
pub(super) fn render(set: &PpuReferenceSet) -> String {
    let pretty = serde_json::to_string_pretty(set).expect("a set serializes");
    let mut out: Vec<String> = Vec::new();
    let mut opens: Vec<usize> = Vec::new();
    for line in pretty.lines() {
        let body = line.trim_start();
        if !(body.starts_with(']') || body.starts_with('}')) {
            if body.ends_with('[') || body.ends_with('{') {
                opens.push(out.len());
            }
            out.push(line.to_owned());
            continue;
        }
        let open = opens.pop().expect("pretty JSON is balanced");
        let inner: Vec<&str> = out[open + 1..]
            .iter()
            .map(|l| l.trim().trim_end_matches(','))
            .collect();
        let plain = inner.iter().all(|l| !l.contains(['[', '{']));
        let array = out[open].ends_with('[');
        if !plain || (!array && inner.len() > 2) {
            out.push(line.to_owned());
            continue;
        }
        let joined = format!("{}{}{body}", out[open], inner.join(", "));
        out.truncate(open);
        out.push(joined);
    }
    out.join("\n") + "\n"
}

/// The file name a unit's set takes: the mnemonic, a record form's `.`
/// spelled `_dot`.
pub(super) fn file_name(unit: &str) -> String {
    format!("{}.json", unit.replace('.', "_dot"))
}
