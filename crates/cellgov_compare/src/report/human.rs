//! Human-readable terminal renderers for single and multi-baseline reports.

use std::fmt::Write;

use crate::compare::{CompareResult, MultiCompareResult};

use super::labels::{classification_label, mode_str};

/// Format a comparison result as human-readable terminal text.
pub fn format_human(result: &CompareResult) -> String {
    let mut out = String::new();

    let class = classification_label(result.classification);
    let mode = mode_str(result.mode);

    let _ = writeln!(out, "classification: {class}");
    let _ = writeln!(out, "mode: {mode}");

    if let Some((expected, actual)) = &result.outcome_mismatch {
        let _ = writeln!(out, "outcome: expected {expected:?}, actual {actual:?}");
    }

    if let Some(d) = &result.memory_divergence {
        let _ = writeln!(
            out,
            "memory: region=\"{}\" offset={} expected=0x{:02x} actual=0x{:02x}",
            d.region, d.offset, d.expected, d.actual
        );
    }

    if let Some(d) = &result.event_divergence {
        let _ = write!(out, "events: index={}", d.index);
        match (&d.expected, &d.actual) {
            (Some(e), Some(a)) => {
                let _ = writeln!(
                    out,
                    " expected={:?}/unit={} actual={:?}/unit={}",
                    e.kind, e.unit, a.kind, a.unit
                );
            }
            (Some(e), None) => {
                let _ = writeln!(
                    out,
                    " expected={:?}/unit={} actual=<missing>",
                    e.kind, e.unit
                );
            }
            (None, Some(a)) => {
                let _ = writeln!(
                    out,
                    " expected=<missing> actual={:?}/unit={}",
                    a.kind, a.unit
                );
            }
            // compare::compare never constructs an EventDivergence with
            // both sides None: every divergence has at least one event.
            (None, None) => unreachable!("EventDivergence with both sides None"),
        }
    }

    out
}

/// Format a multi-baseline comparison result as human-readable terminal text.
pub fn format_multi_human(result: &MultiCompareResult, baseline_count: usize) -> String {
    let mut out = String::new();

    let class = classification_label(result.classification);

    let _ = writeln!(out, "classification: {class}");
    let _ = writeln!(out, "mode: {}", mode_str(result.mode));
    let _ = writeln!(out, "baselines: {baseline_count}");

    if let Some(ref div) = result.oracle_divergence {
        let _ = writeln!(out, "oracle: DISAGREE");
        let _ = write!(out, "{}", format_human(div));
    } else {
        let _ = writeln!(out, "oracle: AGREE");
        if let Some(ref cg) = result.cellgov_result {
            let _ = write!(out, "{}", format_human(cg));
        }
    }

    out
}

#[cfg(test)]
mod tests {
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
}
