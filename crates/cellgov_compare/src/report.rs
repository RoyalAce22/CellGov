//! Human-readable and machine-readable comparison reports.
//!
//! Formats a `CompareResult` (plus the two observations that produced it)
//! for terminal display or JSON serialization. The comparison layer
//! produces structured divergence data; this module turns it into output.

use crate::compare::{Classification, CompareMode, CompareResult, MultiCompareResult};
use crate::observation::Observation;
use serde::Serialize;
use std::fmt::Write;

/// Format a comparison result as human-readable text for terminal output.
///
/// Includes classification, mode, and first-divergence details when the
/// result is a divergence. Does not include full observations -- use
/// `format_json` for offline diffing.
pub fn format_human(result: &CompareResult) -> String {
    let mut out = String::new();

    let class = classification_label(result.classification);
    let mode = mode_str(result.mode);

    writeln!(out, "classification: {class}").ok();
    writeln!(out, "mode: {mode}").ok();

    if let Some((expected, actual)) = &result.outcome_mismatch {
        writeln!(out, "outcome: expected {expected:?}, actual {actual:?}").ok();
    }

    if let Some(d) = &result.memory_divergence {
        writeln!(
            out,
            "memory: region=\"{}\" offset={} expected=0x{:02x} actual=0x{:02x}",
            d.region, d.offset, d.expected, d.actual
        )
        .ok();
    }

    if let Some(d) = &result.event_divergence {
        write!(out, "events: index={}", d.index).ok();
        match (&d.expected, &d.actual) {
            (Some(e), Some(a)) => {
                writeln!(
                    out,
                    " expected={:?}/unit={} actual={:?}/unit={}",
                    e.kind, e.unit, a.kind, a.unit
                )
                .ok();
            }
            (Some(e), None) => {
                writeln!(
                    out,
                    " expected={:?}/unit={} actual=<missing>",
                    e.kind, e.unit
                )
                .ok();
            }
            (None, Some(a)) => {
                writeln!(
                    out,
                    " expected=<missing> actual={:?}/unit={}",
                    a.kind, a.unit
                )
                .ok();
            }
            (None, None) => {
                writeln!(out).ok();
            }
        }
    }

    out
}

/// Machine-readable JSON report with both full observations embedded
/// for offline diffing.
///
/// The `expected` observation is typically the oracle (RPCS3 or saved
/// baseline). The `actual` observation is typically CellGov.
pub fn format_json(
    result: &CompareResult,
    expected: &Observation,
    actual: &Observation,
) -> Result<String, serde_json::Error> {
    let report = JsonReport {
        classification: classification_slug(result.classification),
        mode: mode_str(result.mode),
        outcome_mismatch: result.outcome_mismatch.map(|(e, a)| OutcomePair {
            expected: format!("{e:?}"),
            actual: format!("{a:?}"),
        }),
        memory_divergence: result.memory_divergence.as_ref().map(|d| MemoryDiv {
            region: &d.region,
            offset: d.offset,
            expected: d.expected,
            actual: d.actual,
        }),
        event_divergence: result.event_divergence.as_ref().map(|d| EventDiv {
            index: d.index,
            expected: d.expected.map(|e| EventRef {
                kind: format!("{:?}", e.kind),
                unit: e.unit,
            }),
            actual: d.actual.map(|a| EventRef {
                kind: format!("{:?}", a.kind),
                unit: a.unit,
            }),
        }),
        expected,
        actual,
    };
    serde_json::to_string_pretty(&report)
}

/// Format a multi-baseline comparison result as human-readable text.
///
/// Reports oracle agreement status and, if settled, the CellGov
/// comparison result.
pub fn format_multi_human(result: &MultiCompareResult, baseline_count: usize) -> String {
    let mut out = String::new();

    let class = classification_label(result.classification);

    writeln!(out, "classification: {class}").ok();
    writeln!(out, "mode: {}", mode_str(result.mode)).ok();
    writeln!(out, "baselines: {baseline_count}").ok();

    if let Some(ref div) = result.oracle_divergence {
        writeln!(out, "oracle: DISAGREE").ok();
        write!(out, "{}", format_human(div)).ok();
    } else {
        writeln!(out, "oracle: AGREE").ok();
        if let Some(ref cg) = result.cellgov_result {
            write!(out, "{}", format_human(cg)).ok();
        }
    }

    out
}

/// Machine-readable JSON for multi-baseline comparison.
pub fn format_multi_json(
    result: &MultiCompareResult,
    baselines: &[Observation],
    cellgov: &Observation,
) -> Result<String, serde_json::Error> {
    let report = MultiJsonReport {
        classification: classification_slug(result.classification),
        mode: mode_str(result.mode),
        baseline_count: baselines.len(),
        oracle_settled: result.oracle_divergence.is_none(),
        cellgov,
    };
    serde_json::to_string_pretty(&report)
}

fn mode_str(mode: CompareMode) -> &'static str {
    match mode {
        CompareMode::Strict => "strict",
        CompareMode::Memory => "memory",
        CompareMode::Events => "events",
        CompareMode::Prefix => "prefix",
    }
}

fn classification_label(c: Classification) -> &'static str {
    match c {
        Classification::Match => "MATCH",
        Classification::Divergence => "DIVERGENCE",
        Classification::Unsupported => "UNSUPPORTED",
        Classification::UnsettledOracle => "UNSETTLED_ORACLE",
    }
}

fn classification_slug(c: Classification) -> &'static str {
    match c {
        Classification::Match => "match",
        Classification::Divergence => "divergence",
        Classification::Unsupported => "unsupported",
        Classification::UnsettledOracle => "unsettled_oracle",
    }
}

#[derive(Serialize)]
struct JsonReport<'a> {
    classification: &'static str,
    mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome_mismatch: Option<OutcomePair>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_divergence: Option<MemoryDiv<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_divergence: Option<EventDiv>,
    expected: &'a Observation,
    actual: &'a Observation,
}

#[derive(Serialize)]
struct OutcomePair {
    expected: String,
    actual: String,
}

#[derive(Serialize)]
struct MemoryDiv<'a> {
    region: &'a str,
    offset: usize,
    expected: u8,
    actual: u8,
}

#[derive(Serialize)]
struct EventDiv {
    index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected: Option<EventRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actual: Option<EventRef>,
}

#[derive(Serialize)]
struct EventRef {
    kind: String,
    unit: u64,
}

#[derive(Serialize)]
struct MultiJsonReport<'a> {
    classification: &'static str,
    mode: &'static str,
    baseline_count: usize,
    oracle_settled: bool,
    cellgov: &'a Observation,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::{EventDivergence, MemoryDivergence};
    use crate::observation::{
        NamedMemoryRegion, ObservedEvent, ObservedEventKind, ObservedOutcome,
    };
    use crate::test_support::obs as obs_full;

    fn obs(outcome: ObservedOutcome) -> Observation {
        obs_full(outcome, vec![], vec![])
    }

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
    fn json_match_report_roundtrips() {
        let result = CompareResult {
            classification: Classification::Match,
            mode: CompareMode::Memory,
            outcome_mismatch: None,
            memory_divergence: None,
            event_divergence: None,
        };
        let a = obs(ObservedOutcome::Completed);
        let json = format_json(&result, &a, &a).expect("json");
        assert!(json.contains("\"match\""));
        assert!(json.contains("\"memory\""));
        // Full observations are embedded.
        assert!(json.contains("\"expected\""));
        assert!(json.contains("\"actual\""));
    }

    #[test]
    fn json_divergence_includes_details() {
        let result = CompareResult {
            classification: Classification::Divergence,
            mode: CompareMode::Strict,
            outcome_mismatch: Some((ObservedOutcome::Completed, ObservedOutcome::Timeout)),
            memory_divergence: Some(MemoryDivergence {
                region: "r".into(),
                offset: 0,
                expected: 1,
                actual: 2,
            }),
            event_divergence: None,
        };
        let mut a = obs(ObservedOutcome::Completed);
        a.memory_regions.push(NamedMemoryRegion {
            name: "r".into(),
            addr: 0x1000,
            data: vec![1],
        });
        let mut b = obs(ObservedOutcome::Timeout);
        b.memory_regions.push(NamedMemoryRegion {
            name: "r".into(),
            addr: 0x1000,
            data: vec![2],
        });
        let json = format_json(&result, &a, &b).expect("json");
        assert!(json.contains("\"divergence\""));
        assert!(json.contains("\"outcome_mismatch\""));
        assert!(json.contains("\"memory_divergence\""));
        // event_divergence should be absent (skip_serializing_if)
        assert!(!json.contains("\"event_divergence\""));
    }

    #[test]
    fn json_is_valid_json() {
        let result = CompareResult {
            classification: Classification::Match,
            mode: CompareMode::Prefix,
            outcome_mismatch: None,
            memory_divergence: None,
            event_divergence: None,
        };
        let a = obs(ObservedOutcome::Completed);
        let json = format_json(&result, &a, &a).expect("json");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(parsed["classification"], "match");
        assert_eq!(parsed["mode"], "prefix");
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
    fn json_unsupported_classification() {
        let result = CompareResult {
            classification: Classification::Unsupported,
            mode: CompareMode::Memory,
            outcome_mismatch: None,
            memory_divergence: None,
            event_divergence: None,
        };
        let a = obs(ObservedOutcome::Completed);
        let json = format_json(&result, &a, &a).expect("json");
        assert!(json.contains("\"unsupported\""));
    }

    // -- multi-baseline report tests --

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

    #[test]
    fn multi_json_settled() {
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
        let a = obs(ObservedOutcome::Completed);
        let json = format_multi_json(&result, std::slice::from_ref(&a), &a).expect("json");
        assert!(json.contains("\"match\""));
        assert!(json.contains("\"oracle_settled\": true"));
        assert!(json.contains("\"baseline_count\": 1"));
    }

    #[test]
    fn multi_json_unsettled() {
        let result = MultiCompareResult {
            classification: Classification::UnsettledOracle,
            mode: CompareMode::Strict,
            oracle_divergence: Some(CompareResult {
                classification: Classification::Divergence,
                mode: CompareMode::Strict,
                outcome_mismatch: None,
                memory_divergence: None,
                event_divergence: None,
            }),
            cellgov_result: None,
        };
        let a = obs(ObservedOutcome::Completed);
        let b = obs(ObservedOutcome::Fault);
        let json =
            format_multi_json(&result, &[a, b], &obs(ObservedOutcome::Completed)).expect("json");
        assert!(json.contains("\"unsettled_oracle\""));
        assert!(json.contains("\"oracle_settled\": false"));
        assert!(json.contains("\"baseline_count\": 2"));
    }
}
