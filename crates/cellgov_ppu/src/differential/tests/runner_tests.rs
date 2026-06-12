//! Differential case-runner outcome classification: pass, state mismatch, decode error.

use super::*;
use crate::differential::{MemorySnapshot, OracleSource};

fn nop_state() -> PpuStateSnapshot {
    PpuStateSnapshot::zero()
}

#[test]
fn ori_nop_passes_with_identity_expected_state() {
    // 0x6000_0000 = ori r0, r0, 0 (canonical PPC nop).
    let case = InstructionCase {
        label: "ori_nop".to_string(),
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
        label: "ori_nop_lying_expected".to_string(),
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
        label: "tdi_unhandled".to_string(),
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
