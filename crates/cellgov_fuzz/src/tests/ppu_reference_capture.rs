//! The converter from a table-probe console capture to PPU reference
//! vector sets: one set per unit, each vector one captured case.
//!
//! A vector starts from the state the case loads. Its expected end takes
//! each recorded slot, CR, XER and FPSCR from the capture, and every
//! other GPR at its start value: the case's words write only the
//! registers the case records. LR, CTR and VRSAVE end at their start
//! values. The exception is one the words change: the case's last word
//! reads it back into a recorded GPR, and that read is its end value.
//! CellGov runs each case for three checks:
//!
//! - its recorded result reads no register the routine keeps;
//! - its words write no GPR the case leaves unrecorded;
//! - its words change no LR, CTR or VRSAVE they do not read back.
//!
//! The converter skips a case that fails a check, and a case whose words
//! branch. No expected value comes from CellGov.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use cellgov_ppu::differential::{execute_state_in, PpuStateSnapshot};
use cellgov_ppu::instruction::PpuInstruction as I;
use cellgov_testkit::console_cases::{capture, parse_table, Capture, Case, Reg, Run, PROFILE};

use super::floor::{features, floor};
use crate::ppu_reference::{
    word_unit, PpuReferenceInputState, PpuReferenceObservation, PpuReferenceProvenance,
    PpuReferenceSet, PpuReferenceState, PpuReferenceStateBase, PpuReferenceStop, PpuReferenceUnit,
    PpuReferenceVector, ReferenceField, MAX_PPU_REFERENCE_VECTORS,
    PPU_REFERENCE_SET_SCHEMA_VERSION,
};

/// The address of the data bytes every reference vector names.
pub(super) const MEMORY_BASE: u64 = 0x1000_0000;

/// The count of data bytes each vector names. A fixed-point case reads and
/// writes none, so they end as they start.
pub(super) const DATA_BYTES: usize = 16;

/// The VSCR a new PPU thread starts with: NJ set.
pub(super) const ENTRY_VSCR: u32 = 0x0001_0000;

/// The GPRs the probe routine keeps for itself: r0 is its scratch, r11
/// and r12 its data and result pointers.
const ROUTINE_GPRS: [usize; 3] = [0, 11, 12];

/// The units this converter fills: the fixed-point instructions, the
/// condition-register logical instructions and moves, then the
/// special-purpose register moves.
pub(super) const UNITS: [&str; 85] = [
    "add", "addc", "adde", "addic", "addic.", "addis", "addme", "addze", "and", "andc", "andi.",
    "andis.", "cmp", "cmpi", "cmpl", "cmpli", "cntlzd", "cntlzw", "divd", "divdu", "divw", "divwu",
    "eqv", "extsb", "extsh", "extsw", "mulhd", "mulhdu", "mulhw", "mulhwu", "mulld", "mulli",
    "mullw", "nand", "neg", "nor", "or", "orc", "ori", "oris", "rldcl", "rldcr", "rldic", "rldicl",
    "rldicr", "rldimi", "rlwimi", "rlwinm", "rlwnm", "sld", "slw", "srad", "sradi", "sraw",
    "srawi", "srd", "srw", "subf", "subfc", "subfe", "subfic", "subfme", "subfze", "td", "tdi",
    "tw", "twi", "xor", "xori", "xoris", "crand", "crandc", "creqv", "crnand", "crnor", "cror",
    "crorc", "crxor", "mcrf", "mfcr", "mfocrf", "mtcrf", "mtocrf", "mfspr", "mtspr",
];

/// The units whose words move the PC somewhere other than the next word.
/// The converter runs a case's words one after another, so it skips a
/// case that holds one.
const FLOW_UNITS: [&str; 5] = ["b", "bc", "bclr", "bcctr", "sc"];

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
    /// A word of the case moves the PC somewhere other than the next word.
    Flow,
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
pub(super) struct CaptureSource {
    pub(super) device: String,
    pub(super) environment: String,
    pub(super) frame_sha256: String,
}

pub(super) fn capture_source(probe: &str) -> CaptureSource {
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

/// The instruction mnemonic of a word's unit, or `None` for a word in a
/// group unit or with no generation recipe.
fn word_mnemonic(word: u32) -> Option<String> {
    match word_unit(word)? {
        PpuReferenceUnit::Instruction { mnemonic } => Some(mnemonic),
        _ => None,
    }
}

/// The units a case covers: each unit of [`UNITS`] one of its words
/// belongs to, in word order, each once.
fn case_units(case: &Case) -> Vec<String> {
    let mut units: Vec<String> = Vec::new();
    for mnemonic in case.words.iter().filter_map(|&w| word_mnemonic(w)) {
        if UNITS.contains(&mnemonic.as_str()) && !units.contains(&mnemonic) {
            units.push(mnemonic);
        }
    }
    units
}

/// Whether a word of the case moves the PC somewhere other than the next
/// word.
fn has_flow(case: &Case) -> bool {
    case.words
        .iter()
        .filter_map(|&w| word_mnemonic(w))
        .any(|m| FLOW_UNITS.contains(&m.as_str()))
}

/// A special-purpose register a case may move a value to and read back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Spr {
    Lr,
    Ctr,
    Vrsave,
}

impl Spr {
    const ALL: [Self; 3] = [Self::Lr, Self::Ctr, Self::Vrsave];

    fn of(self, s: &PpuStateSnapshot) -> u64 {
        match self {
            Self::Lr => s.lr,
            Self::Ctr => s.ctr,
            Self::Vrsave => u64::from(s.vrsave),
        }
    }

    /// The GPR a word moves this register into, when it is that move.
    fn read_into(self, word: u32) -> Option<u8> {
        match (self, cellgov_ppu::decode::decode(word).ok()?) {
            (Self::Lr, I::Mflr { rt })
            | (Self::Ctr, I::Mfctr { rt })
            | (Self::Vrsave, I::Mfvrsave { rt }) => Some(rt),
            _ => None,
        }
    }
}

/// The console's value of each special-purpose register the case's words
/// change, read from the GPR its last word moves it into. `None` when the
/// words change one that no recorded read gives back.
fn spr_readbacks(
    case: &Case,
    start: &PpuStateSnapshot,
    end: &PpuStateSnapshot,
    run: &Run,
) -> Option<Vec<(Spr, u64)>> {
    let last = *case.words.last()?;
    let mut out = Vec::new();
    for spr in Spr::ALL {
        if spr.of(start) == spr.of(end) {
            continue;
        }
        let rt = spr.read_into(last)?;
        let slot = case
            .slots()
            .into_iter()
            .find_map(|(r, slot)| (r == Reg::Gpr(rt)).then_some(slot))?;
        out.push((spr, run.slots[slot]));
    }
    Some(out)
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
///
/// The routine registers are its GPRs, and LR, CTR and VRSAVE. The
/// console holds these three at values of its own when the words start.
fn perturbed(start: &PpuStateSnapshot) -> PpuStateSnapshot {
    let mut s = start.clone();
    for n in ROUTINE_GPRS {
        s.gpr[n] = 0xa5a5_0000_0000_0000 | n as u64;
    }
    s.lr = 0xa5a5_0000_0000_0108;
    s.ctr = 0xa5a5_0000_0000_0109;
    s.vrsave = 0xa5a5_0100;
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

/// The special-purpose register values a converted case ends with, or
/// why the case is left out.
fn convertible(
    case: &Case,
    start: &PpuStateSnapshot,
    runs: &[Run; 2],
) -> Result<Vec<(Spr, u64)>, Skip> {
    if case_units(case).is_empty() {
        return Err(Skip::OtherUnit);
    }
    if has_flow(case) {
        return Err(Skip::Flow);
    }
    if case.mem.is_some() {
        return Err(Skip::Memory);
    }
    if runs[0] != runs[1] {
        return Err(Skip::Varies);
    }
    let end = cellgov_end(case, start).ok_or(Skip::Refused)?;
    // A refusal only under other routine registers is a dependence on them.
    let moved = cellgov_end(case, &perturbed(start)).ok_or(Skip::ReadsRoutineRegister)?;
    if recorded(case, &end) != recorded(case, &moved) {
        return Err(Skip::ReadsRoutineRegister);
    }
    if !writes_only_recorded(case, start, &end) {
        return Err(Skip::WritesUnrecorded);
    }
    spr_readbacks(case, start, &end, &runs[0]).ok_or(Skip::WritesUnrecorded)
}

pub(super) fn value<T>(value: T) -> ReferenceField<T> {
    ReferenceField::Value { value }
}

pub(super) fn unsupported<T>(reason: &str) -> ReferenceField<T> {
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
pub(super) fn unobserved_stop() -> PpuReferenceStop {
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
fn expected(
    case: &Case,
    start: &PpuStateSnapshot,
    run: &Run,
    sprs: &[(Spr, u64)],
) -> PpuReferenceObservation {
    const RUNTIME: &str = "the console capture does not observe the runtime's effect pipeline";
    let words = case.words.len() as u64;
    let spr = |which: Spr| {
        sprs.iter()
            .find_map(|&(s, v)| (s == which).then_some(v))
            .unwrap_or_else(|| which.of(start))
    };
    PpuReferenceObservation {
        state: PpuReferenceState {
            gpr: value(expected_gprs(case, start, run)),
            fpr: unsupported("the fixed-point case records no floating-point register"),
            vr_hex: unsupported("the fixed-point case records no vector register"),
            pc: value(4 * words),
            cr: value(run.cr),
            lr: value(spr(Spr::Lr)),
            ctr: value(spr(Spr::Ctr)),
            xer: value(run.xer),
            fpscr: value(run.fpscr),
            vrsave: value(spr(Spr::Vrsave) as u32),
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
    sprs: &[(Spr, u64)],
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
        expected: expected(case, start, run, sprs),
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
                let skip = if !case_units(case).is_empty() {
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
        let sprs = match convertible(case, &start, runs) {
            Ok(sprs) => sprs,
            Err(skip) => {
                *out.skipped.entry(skip).or_default() += 1;
                continue;
            }
        };
        let converted = vector(probe, &source, case, &start, &runs[0], &sprs);
        for unit in case_units(case) {
            out.candidates
                .entry(unit)
                .or_default()
                .push(converted.clone());
        }
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
    let shown: Vec<BTreeSet<String>> = candidates
        .iter()
        .map(|v| features(&floor, unit, v))
        .collect();
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
