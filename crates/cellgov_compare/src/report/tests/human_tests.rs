//! Human-readable report rendering for each classification and divergence detail kind.

use super::*;
use crate::compare::{
    Classification, CompareMode, CompareResult, EventDivergence, MemoryDivergence,
    MultiCompareResult,
};
use crate::observation::{ObservedEvent, ObservedEventKind, ObservedOutcome};

#[test]
fn human_match_report() {
    let result = CompareResult {
        classification: Classification::Match,
        mode: CompareMode::Memory,
        outcome_mismatch: None,
        memory_divergence: None,
        event_divergence: None,
    };
    let text = format_human(&result);
    assert!(text.contains("MATCH"));
    assert!(text.contains("memory"));
    assert!(!text.contains("outcome:"));
}

#[test]
fn human_divergence_with_outcome() {
    let result = CompareResult {
        classification: Classification::Divergence,
        mode: CompareMode::Strict,
        outcome_mismatch: Some((ObservedOutcome::Completed, ObservedOutcome::Fault)),
        memory_divergence: None,
        event_divergence: None,
    };
    let text = format_human(&result);
    assert!(text.contains("DIVERGENCE"));
    assert!(text.contains("Completed"));
    assert!(text.contains("Fault"));
}

#[test]
fn human_divergence_with_memory() {
    let result = CompareResult {
        classification: Classification::Divergence,
        mode: CompareMode::Memory,
        outcome_mismatch: None,
        memory_divergence: Some(MemoryDivergence {
            region: "result".into(),
            offset: 3,
            expected: 0xAA,
            actual: 0xBB,
        }),
        event_divergence: None,
    };
    let text = format_human(&result);
    assert!(text.contains("region=\"result\""));
    assert!(text.contains("offset=3"));
    assert!(text.contains("0xaa"));
    assert!(text.contains("0xbb"));
}

#[test]
fn human_divergence_with_events() {
    let result = CompareResult {
        classification: Classification::Divergence,
        mode: CompareMode::Events,
        outcome_mismatch: None,
        memory_divergence: None,
        event_divergence: Some(EventDivergence {
            index: 1,
            expected: Some(ObservedEvent {
                kind: ObservedEventKind::MailboxSend,
                unit: 0,
                sequence: 1,
            }),
            actual: Some(ObservedEvent {
                kind: ObservedEventKind::UnitBlock,
                unit: 2,
                sequence: 1,
            }),
        }),
    };
    let text = format_human(&result);
    assert!(text.contains("index=1"));
    assert!(text.contains("MailboxSend"));
    assert!(text.contains("UnitBlock"));
}

#[test]
fn human_event_divergence_with_missing_actual() {
    let result = CompareResult {
        classification: Classification::Divergence,
        mode: CompareMode::Strict,
        outcome_mismatch: None,
        memory_divergence: None,
        event_divergence: Some(EventDivergence {
            index: 2,
            expected: Some(ObservedEvent {
                kind: ObservedEventKind::DmaComplete,
                unit: 0,
                sequence: 2,
            }),
            actual: None,
        }),
    };
    let text = format_human(&result);
    assert!(text.contains("<missing>"));
}

#[test]
fn human_unsupported_report() {
    let result = CompareResult {
        classification: Classification::Unsupported,
        mode: CompareMode::Memory,
        outcome_mismatch: None,
        memory_divergence: None,
        event_divergence: None,
    };
    let text = format_human(&result);
    assert!(text.contains("UNSUPPORTED"));
}

#[test]
fn human_unsettled_oracle_report() {
    let result = CompareResult {
        classification: Classification::UnsettledOracle,
        mode: CompareMode::Strict,
        outcome_mismatch: None,
        memory_divergence: None,
        event_divergence: None,
    };
    let text = format_human(&result);
    assert!(text.contains("UNSETTLED_ORACLE"));
}

#[test]
fn multi_human_match() {
    let result = MultiCompareResult {
        classification: Classification::Match,
        mode: CompareMode::Memory,
        oracle_divergence: None,
        cellgov_result: Some(CompareResult {
            classification: Classification::Match,
            mode: CompareMode::Memory,
            outcome_mismatch: None,
            memory_divergence: None,
            event_divergence: None,
        }),
    };
    let text = format_multi_human(&result, 2);
    assert!(text.contains("MATCH"));
    assert!(text.contains("baselines: 2"));
    assert!(text.contains("oracle: AGREE"));
}

#[test]
fn multi_human_unsettled() {
    let result = MultiCompareResult {
        classification: Classification::UnsettledOracle,
        mode: CompareMode::Strict,
        oracle_divergence: Some(CompareResult {
            classification: Classification::Divergence,
            mode: CompareMode::Strict,
            outcome_mismatch: Some((ObservedOutcome::Completed, ObservedOutcome::Fault)),
            memory_divergence: None,
            event_divergence: None,
        }),
        cellgov_result: None,
    };
    let text = format_multi_human(&result, 2);
    assert!(text.contains("UNSETTLED_ORACLE"));
    assert!(text.contains("oracle: DISAGREE"));
    assert!(text.contains("Completed"));
    assert!(text.contains("Fault"));
}
