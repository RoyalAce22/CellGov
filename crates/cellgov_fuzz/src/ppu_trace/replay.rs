//! The replay of a trace's records, the per-kind counts, and the class
//! of each failing record.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_ppu::differential::rpcs3_capture::CapturedRecord;
use cellgov_ppu::differential::{
    execute_state_in, is_context_dependent, run_case, CaseOutcome, ControlRegisters,
    PpuStateSnapshot, StateDiff,
};
use cellgov_ppu::instruction::fuzz::{
    expected_generation_kinds, generation_descriptor, PpuFuzzKind,
};

use super::console::{ConsoleSide, ConsoleVector};
use crate::ppu_documented::{open_places, OpenAt, OpenPlace, Set};

/// The capture id every replayed case carries.
const CAPTURE_ID: &str = "rpcs3-trace";

/// The kind name of a word no generation recipe covers.
const UNDECODED: &str = "undecoded";

/// How a replay treats the records.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PpuTraceOptions {
    /// Count a context-dependent record as skipped instead of running it.
    pub skip_context_dependent: bool,
}

/// One decoded kind's counts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PpuTraceKindRow {
    /// The kind's name.
    pub kind: String,
    /// Records of the kind.
    pub seen: usize,
    /// Records CellGov reproduced.
    pub passed: usize,
    /// Records CellGov did not reproduce.
    pub failed: usize,
    /// Records the replay did not run.
    pub skipped: usize,
}

/// Why a record failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PpuTraceClass {
    /// Every differing bit lies where the documents leave the result
    /// undefined.
    Undefined {
        /// The documented sets that reach those bits.
        sets: Vec<Set>,
    },
    /// A console vector with the record's inputs agrees with RPCS3.
    CellGovDefect {
        /// The vector, as `<file> <name>`.
        vector: String,
    },
    /// A console vector with the record's inputs agrees with CellGov.
    Rpcs3Divergence {
        /// The vector, as `<file> <name>`.
        vector: String,
    },
    /// The outcome depends on state one instruction's replay cannot
    /// restore.
    ContextDependent,
    /// No other class fits.
    Unclassified,
}

impl PpuTraceClass {
    /// The class's stable name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Undefined { .. } => "undefined",
            Self::CellGovDefect { .. } => "cellgov-defect",
            Self::Rpcs3Divergence { .. } => "rpcs3-divergence",
            Self::ContextDependent => "context-dependent",
            Self::Unclassified => "unclassified",
        }
    }

    /// Whether the class asks for a person: a CellGov defect, or no
    /// class at all.
    pub fn needs_triage(&self) -> bool {
        matches!(self, Self::CellGovDefect { .. } | Self::Unclassified)
    }
}

/// One failing record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuTraceFailure {
    /// The record's index in the trace.
    pub record: usize,
    /// The record's program counter.
    pub pc: u64,
    /// The record's instruction word.
    pub raw: u32,
    /// The word's kind name.
    pub kind: String,
    /// What the replay found.
    pub outcome: CaseOutcome,
    /// Why it failed.
    pub class: PpuTraceClass,
}

/// A whole trace's replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuTraceReport {
    /// Records in the trace.
    pub records: usize,
    /// Whether the trace recorded VRSAVE, the FPSCR and the VSCR, or
    /// `None` for a trace with no record.
    pub control_registers: Option<ControlRegisters>,
    /// Every kind the trace reached, by name.
    pub kinds: Vec<PpuTraceKindRow>,
    /// Every decoded kind the trace never reached, by name.
    pub unreached: Vec<String>,
    /// Every failing record, in trace order.
    pub failures: Vec<PpuTraceFailure>,
}

impl PpuTraceReport {
    /// Failures whose class needs triage.
    pub fn triage_count(&self) -> usize {
        self.failures
            .iter()
            .filter(|f| f.class.needs_triage())
            .count()
    }
}

/// The name a report gives `kind`.
pub fn kind_name(kind: PpuFuzzKind) -> String {
    format!("{kind:?}")
}

/// Replays every record, counts each kind, and classifies each failure
/// against the documents and `console`.
pub fn replay_trace(
    records: &[CapturedRecord],
    console: &[ConsoleVector],
    options: PpuTraceOptions,
) -> PpuTraceReport {
    let mut kinds: BTreeMap<String, PpuTraceKindRow> = BTreeMap::new();
    let mut reached = BTreeSet::new();
    let mut failures = Vec::new();
    for (index, record) in records.iter().enumerate() {
        let kind = generation_descriptor(record.raw_instruction).map(|d| d.kind);
        reached.extend(kind);
        let name = kind.map_or_else(|| UNDECODED.to_owned(), kind_name);
        let row = kinds
            .entry(name.clone())
            .or_insert_with(|| PpuTraceKindRow {
                kind: name.clone(),
                ..PpuTraceKindRow::default()
            });
        row.seen += 1;
        let context = is_context_dependent(record.raw_instruction);
        if options.skip_context_dependent && context {
            row.skipped += 1;
            continue;
        }
        let case = record.to_instruction_case(format!("record {index}"), CAPTURE_ID);
        match run_case(&case) {
            CaseOutcome::Pass => row.passed += 1,
            outcome => {
                row.failed += 1;
                let class = classify(record, &outcome, context, console);
                failures.push(PpuTraceFailure {
                    record: index,
                    pc: record.pc,
                    raw: record.raw_instruction,
                    kind: name,
                    outcome,
                    class,
                });
            }
        }
    }
    let unreached = expected_generation_kinds()
        .into_iter()
        .filter(|kind| !reached.contains(kind))
        .map(kind_name)
        .collect();
    PpuTraceReport {
        records: records.len(),
        control_registers: records.first().map(|r| r.control_registers),
        kinds: kinds.into_values().collect(),
        unreached,
        failures,
    }
}

/// The first class that fits a failing record.
fn classify(
    record: &CapturedRecord,
    outcome: &CaseOutcome,
    context: bool,
    console: &[ConsoleVector],
) -> PpuTraceClass {
    let cellgov = execute_state_in(
        record.raw_instruction,
        &record.pre_state,
        record.mem_addr,
        record.mem_pre.clone(),
    )
    .ok();
    if let Some((state, memory)) = &cellgov {
        let places = open_places(
            record.raw_instruction,
            &record.pre_state,
            [&record.post_state, state],
        );
        if let Some(sets) = undefined_sets(&places, outcome, record, memory) {
            return PpuTraceClass::Undefined { sets };
        }
        if let Some(class) = console_class(record, (state, memory), console) {
            return class;
        }
    }
    if context {
        PpuTraceClass::ContextDependent
    } else {
        PpuTraceClass::Unclassified
    }
}

/// The class the first console vector that decides between the sides
/// gives, or `None` when no vector decides.
fn console_class(
    record: &CapturedRecord,
    cellgov: (&PpuStateSnapshot, &[u8]),
    console: &[ConsoleVector],
) -> Option<PpuTraceClass> {
    let rpcs3 = (&record.post_state, record.mem_post.as_slice());
    for vector in console.iter().filter(|v| v.matches(record)) {
        let named = format!("{} {}", vector.file, vector.name);
        let Some(side) = vector.decide(rpcs3, cellgov, record.mem_addr, record.control_registers)
        else {
            continue;
        };
        return Some(match side {
            ConsoleSide::Rpcs3 => PpuTraceClass::CellGovDefect { vector: named },
            ConsoleSide::CellGov => PpuTraceClass::Rpcs3Divergence { vector: named },
        });
    }
    None
}

/// The sets of `places` when they hold every bit the two sides disagree
/// on, or `None` when a differing bit lies outside them.
fn undefined_sets(
    places: &[OpenPlace],
    outcome: &CaseOutcome,
    record: &CapturedRecord,
    cellgov_memory: &[u8],
) -> Option<Vec<Set>> {
    if places.is_empty() {
        return None;
    }
    let state_open = match outcome {
        CaseOutcome::StateMismatch(diff) => state_is_open(places, diff),
        CaseOutcome::Pass | CaseOutcome::MemoryMismatch(_) => true,
        CaseOutcome::DecodeError(_) | CaseOutcome::UnexpectedVerdict(_) => false,
    };
    // A state mismatch hides the memory comparison, so this check
    // compares the bytes for every outcome.
    let memory_open = record
        .mem_post
        .iter()
        .zip(cellgov_memory)
        .enumerate()
        .filter(|(_, (rpcs3, cellgov))| rpcs3 != cellgov)
        .all(|(offset, _)| byte_is_open(places, record.mem_addr.wrapping_add(offset as u64)));
    let sets: BTreeSet<Set> = places.iter().map(|place| place.set).collect();
    (state_open && memory_open).then(|| sets.into_iter().collect())
}

/// Whether every differing bit of `diff` lies in `places`.
fn state_is_open(places: &[OpenPlace], diff: &StateDiff) -> bool {
    // No `..`: a new diff field fails to compile until this check judges it.
    let StateDiff {
        gpr,
        fpr,
        vr,
        cr,
        lr,
        ctr,
        xer,
        fpscr,
        vrsave,
        vscr,
        reservation,
    } = diff;
    let open = |pick: &dyn Fn(OpenAt) -> Option<u128>| {
        places
            .iter()
            .filter_map(|p| pick(p.at))
            .fold(0, |acc, m| acc | m)
    };
    let held = |bits: u128, pick: &dyn Fn(OpenAt) -> Option<u128>| bits & !open(pick) == 0;
    let gpr = gpr.iter().all(|&(i, e, o)| {
        held(u128::from(e ^ o), &|at| match at {
            OpenAt::Gpr { index, mask } if usize::from(index) == i => Some(u128::from(mask)),
            _ => None,
        })
    });
    let fpr = fpr.iter().all(|&(i, e, o)| {
        held(u128::from(e ^ o), &|at| match at {
            OpenAt::Fpr { index, mask } if usize::from(index) == i => Some(u128::from(mask)),
            _ => None,
        })
    });
    let vr = vr.iter().all(|&(i, e, o)| {
        held(e ^ o, &|at| match at {
            OpenAt::Vr { index, mask } if usize::from(index) == i => Some(mask),
            _ => None,
        })
    });
    let word = |pair: Option<(u32, u32)>, pick: &dyn Fn(OpenAt) -> Option<u128>| {
        pair.is_none_or(|(e, o)| held(u128::from(e ^ o), pick))
    };
    let cr = word(*cr, &|at| match at {
        OpenAt::Cr(bits) => Some(u128::from(bits)),
        _ => None,
    });
    let fpscr = word(*fpscr, &|at| match at {
        OpenAt::Fpscr(bits) => Some(u128::from(bits)),
        _ => None,
    });
    let xer = xer.is_none_or(|(e, o)| {
        held(u128::from(e ^ o), &|at| match at {
            OpenAt::Xer(bits) => Some(u128::from(bits)),
            _ => None,
        })
    });
    let reservation = reservation.is_none() || places.iter().any(|p| p.at == OpenAt::Reservation);
    let closed = lr.is_none() && ctr.is_none() && vrsave.is_none() && vscr.is_none();
    gpr && fpr && vr && cr && fpscr && xer && reservation && closed
}

/// Whether the byte at `addr` lies in an open storage place.
fn byte_is_open(places: &[OpenPlace], addr: u64) -> bool {
    places.iter().any(|p| match p.at {
        OpenAt::Memory { ea, len } => addr.wrapping_sub(ea) < len,
        _ => false,
    })
}

/// The fields a failure names: `gpr3`, `cr`, `mem+0x4`, and so on, or
/// the decode refusal or verdict.
pub fn differing_fields(outcome: &CaseOutcome) -> Vec<String> {
    match outcome {
        CaseOutcome::Pass => Vec::new(),
        CaseOutcome::DecodeError(error) => vec![format!("decode: {error}")],
        CaseOutcome::UnexpectedVerdict(verdict) => vec![format!("verdict: {verdict:?}")],
        CaseOutcome::MemoryMismatch(bytes) => bytes
            .iter()
            .map(|byte| format!("mem+0x{:x}", byte.offset))
            .collect(),
        CaseOutcome::StateMismatch(diff) => {
            // No `..`: a new diff field fails to compile until this list names it.
            let StateDiff {
                gpr,
                fpr,
                vr,
                cr,
                lr,
                ctr,
                xer,
                fpscr,
                vrsave,
                vscr,
                reservation,
            } = diff;
            let mut fields: Vec<String> = Vec::new();
            fields.extend(gpr.iter().map(|(i, ..)| format!("gpr{i}")));
            fields.extend(fpr.iter().map(|(i, ..)| format!("fpr{i}")));
            fields.extend(vr.iter().map(|(i, ..)| format!("vr{i}")));
            let named = [
                ("cr", cr.is_some()),
                ("lr", lr.is_some()),
                ("ctr", ctr.is_some()),
                ("xer", xer.is_some()),
                ("fpscr", fpscr.is_some()),
                ("vrsave", vrsave.is_some()),
                ("vscr", vscr.is_some()),
                ("reservation", reservation.is_some()),
            ];
            fields.extend(
                named
                    .into_iter()
                    .filter(|&(_, differs)| differs)
                    .map(|(name, _)| name.to_owned()),
            );
            fields
        }
    }
}
