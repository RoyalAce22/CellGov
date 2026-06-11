//! Case runner: load, execute, diff, and report.

use cellgov_effects::Effect;
use cellgov_event::UnitId;

use crate::decode::decode;
use crate::exec::{execute, ExecuteVerdict};
use crate::instruction::PpuDecodeError;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

use super::{InstructionCase, PpuStateSnapshot};

/// One byte position diverging between expected and observed memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryByteMismatch {
    /// Offset into the memory snapshot.
    pub offset: usize,
    /// Byte the case expected.
    pub expected: u8,
    /// Byte the executor produced.
    pub observed: u8,
}

/// Per-field divergences between expected and observed register
/// snapshots.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StateDiff {
    /// `(index, expected, observed)` for each diverging GPR.
    pub gpr: Vec<(usize, u64, u64)>,
    /// `(index, expected, observed)` for each diverging FPR.
    pub fpr: Vec<(usize, u64, u64)>,
    /// `(index, expected, observed)` for each diverging VR.
    pub vr: Vec<(usize, u128, u128)>,
    /// `(expected, observed)` if CR diverged.
    pub cr: Option<(u32, u32)>,
    /// `(expected, observed)` if LR diverged.
    pub lr: Option<(u64, u64)>,
    /// `(expected, observed)` if CTR diverged.
    pub ctr: Option<(u64, u64)>,
    /// `(expected, observed)` if XER diverged.
    pub xer: Option<(u64, u64)>,
    /// `(expected, observed)` if reservation state diverged. Each
    /// side renders as the reserved line address (`u64`) or `None`.
    pub reservation: Option<(Option<u64>, Option<u64>)>,
}

impl StateDiff {
    /// True when no fields diverged.
    pub fn is_empty(&self) -> bool {
        self.gpr.is_empty()
            && self.fpr.is_empty()
            && self.vr.is_empty()
            && self.cr.is_none()
            && self.lr.is_none()
            && self.ctr.is_none()
            && self.xer.is_none()
            && self.reservation.is_none()
    }
}

/// Outcome of running an [`InstructionCase`] through the harness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaseOutcome {
    /// Post-state and memory match the expected values.
    Pass,
    /// The decoder rejected the raw instruction.
    DecodeError(PpuDecodeError),
    /// Executor returned a non-`Continue` verdict; the harness treats
    /// any non-`Continue` outcome as a divergence.
    UnexpectedVerdict(ExecuteVerdict),
    /// Register state diverged from the expected snapshot.
    StateMismatch(StateDiff),
    /// Memory bytes diverged from the expected snapshot.
    MemoryMismatch(Vec<MemoryByteMismatch>),
}

/// Run one case and return the [`CaseOutcome`].
///
/// Staged [`Effect::SharedWriteIntent`]s are applied to a private
/// copy of the case's memory before the post-state and that memory
/// are diffed against the expected snapshots.
pub fn run_case(case: &InstructionCase) -> CaseOutcome {
    let mut state = PpuState::new();
    case.initial_state.apply(&mut state);

    let mut memory = case.initial_memory.bytes.clone();
    let base = case.initial_memory.base;

    let inst = match decode(case.raw_instruction) {
        Ok(i) => i,
        Err(e) => return CaseOutcome::DecodeError(e),
    };

    let mut effects: Vec<Effect> = Vec::new();
    let mut store_buf = StoreBuffer::new();
    let verdict = {
        let views: [(u64, &[u8]); 1] = [(base, &memory)];
        execute(
            &inst,
            &mut state,
            UnitId::new(0),
            &views,
            &mut effects,
            &mut store_buf,
        )
    };
    store_buf.flush(&mut effects, UnitId::new(0));

    if verdict != ExecuteVerdict::Continue {
        return CaseOutcome::UnexpectedVerdict(verdict);
    }

    apply_shared_writes(&mut memory, base, &effects);

    let observed_state = PpuStateSnapshot::capture(&state);
    let state_diff = diff_state(&case.expected_state, &observed_state);
    if !state_diff.is_empty() {
        return CaseOutcome::StateMismatch(state_diff);
    }

    let mem_diff = diff_memory(&case.expected_memory.bytes, &memory);
    if !mem_diff.is_empty() {
        return CaseOutcome::MemoryMismatch(mem_diff);
    }

    CaseOutcome::Pass
}

/// Run `case`, asserting a [`CaseOutcome::Pass`].
///
/// # Panics
///
/// On any non-`Pass` outcome, with a diagnostic naming the case
/// label, the source tag, and the divergent fields or byte offsets.
#[track_caller]
pub fn assert_case(case: &InstructionCase) {
    match run_case(case) {
        CaseOutcome::Pass => {}
        CaseOutcome::DecodeError(e) => panic!(
            "differential case '{}' [{:?}]: decoder rejected raw=0x{:08x}: {}",
            case.label, case.source, case.raw_instruction, e
        ),
        CaseOutcome::UnexpectedVerdict(v) => panic!(
            "differential case '{}' [{:?}]: executor returned {:?}, expected Continue",
            case.label, case.source, v
        ),
        CaseOutcome::StateMismatch(diff) => panic!(
            "differential case '{}' [{:?}]: state mismatch:\n{}",
            case.label,
            case.source,
            format_state_diff(&diff)
        ),
        CaseOutcome::MemoryMismatch(diffs) => panic!(
            "differential case '{}' [{:?}]: memory mismatch ({} byte(s)):\n{}",
            case.label,
            case.source,
            diffs.len(),
            format_memory_diff(&diffs)
        ),
    }
}

/// Aggregate result of [`run_corpus`].
#[derive(Debug, Default, Clone)]
pub struct CorpusReport {
    /// Number of cases that returned [`CaseOutcome::Pass`].
    pub passed: usize,
    /// Per-failing-case `(label, outcome)` pairs in input order.
    pub failed: Vec<(&'static str, CaseOutcome)>,
}

impl CorpusReport {
    /// True when every case passed.
    pub fn is_clean(&self) -> bool {
        self.failed.is_empty()
    }

    /// Total cases run.
    pub fn total(&self) -> usize {
        self.passed + self.failed.len()
    }
}

/// Run every case in `cases` and aggregate results.
pub fn run_corpus(cases: &[InstructionCase]) -> CorpusReport {
    let mut report = CorpusReport::default();
    for case in cases {
        match run_case(case) {
            CaseOutcome::Pass => report.passed += 1,
            other => report.failed.push((case.label, other)),
        }
    }
    report
}

/// Walk `effects` and apply each [`Effect::SharedWriteIntent`] that
/// falls inside the `[base, base + memory.len())` window to
/// `memory`. Writes straddling the window edge are clamped to the
/// in-window portion; out-of-window writes are silently dropped so
/// the harness stays robust to executors that touch addresses the
/// case did not map.
fn apply_shared_writes(memory: &mut [u8], base: u64, effects: &[Effect]) {
    let mem_end = base.saturating_add(memory.len() as u64);
    for effect in effects {
        if let Effect::SharedWriteIntent { range, bytes, .. } = effect {
            let write_start = range.start().raw();
            let write_end = write_start.saturating_add(range.length());
            if write_end <= base || write_start >= mem_end {
                continue;
            }
            let clamp_start = write_start.max(base);
            let clamp_end = write_end.min(mem_end);
            let mem_offset = (clamp_start - base) as usize;
            let src_offset = (clamp_start - write_start) as usize;
            let length = (clamp_end - clamp_start) as usize;
            let src = bytes.bytes();
            if src_offset + length <= src.len() {
                memory[mem_offset..mem_offset + length]
                    .copy_from_slice(&src[src_offset..src_offset + length]);
            }
        }
    }
}

/// Field-by-field diff between two [`PpuStateSnapshot`]s.
fn diff_state(expected: &PpuStateSnapshot, observed: &PpuStateSnapshot) -> StateDiff {
    let mut diff = StateDiff::default();
    for i in 0..expected.gpr.len() {
        if expected.gpr[i] != observed.gpr[i] {
            diff.gpr.push((i, expected.gpr[i], observed.gpr[i]));
        }
    }
    for i in 0..expected.fpr.len() {
        if expected.fpr[i] != observed.fpr[i] {
            diff.fpr.push((i, expected.fpr[i], observed.fpr[i]));
        }
    }
    for i in 0..expected.vr.len() {
        if expected.vr[i] != observed.vr[i] {
            diff.vr.push((i, expected.vr[i], observed.vr[i]));
        }
    }
    if expected.cr != observed.cr {
        diff.cr = Some((expected.cr, observed.cr));
    }
    if expected.lr != observed.lr {
        diff.lr = Some((expected.lr, observed.lr));
    }
    if expected.ctr != observed.ctr {
        diff.ctr = Some((expected.ctr, observed.ctr));
    }
    if expected.xer != observed.xer {
        diff.xer = Some((expected.xer, observed.xer));
    }
    if expected.reservation != observed.reservation {
        diff.reservation = Some((
            expected.reservation.map(|r| r.addr()),
            observed.reservation.map(|r| r.addr()),
        ));
    }
    diff
}

/// Byte-by-byte diff between expected and observed memory snapshots.
/// Returns the first 64 divergences to keep panic messages bounded.
fn diff_memory(expected: &[u8], observed: &[u8]) -> Vec<MemoryByteMismatch> {
    let mut diffs = Vec::new();
    let limit = expected.len().min(observed.len());
    for i in 0..limit {
        if expected[i] != observed[i] {
            diffs.push(MemoryByteMismatch {
                offset: i,
                expected: expected[i],
                observed: observed[i],
            });
            if diffs.len() >= 64 {
                break;
            }
        }
    }
    if expected.len() != observed.len() {
        diffs.push(MemoryByteMismatch {
            offset: limit,
            expected: expected.len() as u8,
            observed: observed.len() as u8,
        });
    }
    diffs
}

fn format_state_diff(diff: &StateDiff) -> String {
    let mut lines = Vec::new();
    for &(i, exp, obs) in &diff.gpr {
        lines.push(format!(
            "  gpr[{i:>2}]: expected 0x{exp:016x}, observed 0x{obs:016x}"
        ));
    }
    for &(i, exp, obs) in &diff.fpr {
        lines.push(format!(
            "  fpr[{i:>2}]: expected 0x{exp:016x}, observed 0x{obs:016x}"
        ));
    }
    for &(i, exp, obs) in &diff.vr {
        lines.push(format!(
            "   vr[{i:>2}]: expected 0x{exp:032x}, observed 0x{obs:032x}"
        ));
    }
    if let Some((exp, obs)) = diff.cr {
        lines.push(format!(
            "       cr: expected 0x{exp:08x}, observed 0x{obs:08x}"
        ));
    }
    if let Some((exp, obs)) = diff.lr {
        lines.push(format!(
            "       lr: expected 0x{exp:016x}, observed 0x{obs:016x}"
        ));
    }
    if let Some((exp, obs)) = diff.ctr {
        lines.push(format!(
            "      ctr: expected 0x{exp:016x}, observed 0x{obs:016x}"
        ));
    }
    if let Some((exp, obs)) = diff.xer {
        lines.push(format!(
            "      xer: expected 0x{exp:016x}, observed 0x{obs:016x}"
        ));
    }
    if let Some((exp, obs)) = diff.reservation {
        let fmt = |o: Option<u64>| match o {
            Some(a) => format!("Some(0x{a:016x})"),
            None => String::from("None"),
        };
        lines.push(format!(
            "    rsrvn: expected {}, observed {}",
            fmt(exp),
            fmt(obs)
        ));
    }
    lines.join("\n")
}

fn format_memory_diff(diffs: &[MemoryByteMismatch]) -> String {
    diffs
        .iter()
        .map(|d| {
            format!(
                "  [+{:#06x}]: expected 0x{:02x}, observed 0x{:02x}",
                d.offset, d.expected, d.observed
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::differential::{MemorySnapshot, OracleSource};

    fn nop_state() -> PpuStateSnapshot {
        PpuStateSnapshot::zero()
    }

    #[test]
    fn ori_nop_passes_with_identity_expected_state() {
        // 0x6000_0000 = ori r0, r0, 0 (canonical PPC nop).
        let case = InstructionCase {
            label: "ori_nop",
            initial_state: nop_state(),
            initial_memory: MemorySnapshot::empty(),
            raw_instruction: 0x6000_0000,
            expected_state: nop_state(),
            expected_memory: MemorySnapshot::empty(),
            source: OracleSource::Manual,
        };
        assert_eq!(run_case(&case), CaseOutcome::Pass);
    }

    #[test]
    fn injected_state_divergence_is_reported() {
        let mut bad = nop_state();
        bad.gpr[3] = 0xDEAD_BEEF;
        let case = InstructionCase {
            label: "ori_nop_lying_expected",
            initial_state: nop_state(),
            initial_memory: MemorySnapshot::empty(),
            raw_instruction: 0x6000_0000,
            expected_state: bad,
            expected_memory: MemorySnapshot::empty(),
            source: OracleSource::Manual,
        };
        match run_case(&case) {
            CaseOutcome::StateMismatch(diff) => {
                assert_eq!(diff.gpr.len(), 1);
                assert_eq!(diff.gpr[0], (3, 0xDEAD_BEEF, 0));
            }
            other => panic!("expected StateMismatch, got {other:?}"),
        }
    }

    #[test]
    fn decoder_rejection_surfaces_as_decode_error() {
        // 0x0800_0000 = `tdi` (primary 2), unhandled by the decoder.
        let case = InstructionCase {
            label: "tdi_unhandled",
            initial_state: nop_state(),
            initial_memory: MemorySnapshot::empty(),
            raw_instruction: 0x0800_0000,
            expected_state: nop_state(),
            expected_memory: MemorySnapshot::empty(),
            source: OracleSource::Manual,
        };
        match run_case(&case) {
            CaseOutcome::DecodeError(_) => {}
            other => panic!("expected DecodeError, got {other:?}"),
        }
    }

    #[test]
    fn empty_corpus_is_clean() {
        let report = run_corpus(&[]);
        assert!(report.is_clean());
        assert_eq!(report.total(), 0);
    }
}
