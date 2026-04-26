//! Human-readable and machine-readable comparison report rendering.

use crate::compare::{Classification, CompareMode, CompareResult, MultiCompareResult};
use crate::observation::{Observation, ObservedEventKind, ObservedOutcome};
use serde::Serialize;
use std::fmt::Write;

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

/// Serialize a comparison result plus both full observations as pretty JSON.
///
/// Top-level shape: `classification`, `mode`, optional `outcome_mismatch`,
/// optional `memory_divergence`, optional `event_divergence`, `expected`,
/// `actual`. Consumers rely on this schema.
pub fn format_json(
    result: &CompareResult,
    expected: &Observation,
    actual: &Observation,
) -> Result<String, serde_json::Error> {
    let report = JsonReport {
        body: build_body(result),
        expected,
        actual,
    };
    serde_json::to_string_pretty(&report)
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

/// Serialize a multi-baseline comparison result as pretty JSON.
///
/// Top-level shape: `classification`, `mode`, `baseline_count`,
/// `oracle_settled`, optional `oracle_divergence` (sub-report fields when
/// baselines disagree), optional `cellgov_result` (sub-report fields when
/// the oracle settled), `cellgov` observation.
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
        oracle_divergence: result.oracle_divergence.as_ref().map(build_body),
        cellgov_result: result.cellgov_result.as_ref().map(build_body),
        cellgov,
    };
    serde_json::to_string_pretty(&report)
}

fn build_body(result: &CompareResult) -> CompareReportBody<'_> {
    CompareReportBody {
        classification: classification_slug(result.classification),
        mode: mode_str(result.mode),
        outcome_mismatch: result
            .outcome_mismatch
            .map(|(expected, actual)| OutcomePair { expected, actual }),
        memory_divergence: result.memory_divergence.as_ref().map(|d| MemoryDiv {
            region: &d.region,
            offset: d.offset,
            expected: d.expected,
            actual: d.actual,
        }),
        event_divergence: result.event_divergence.as_ref().map(|d| EventDiv {
            index: d.index,
            expected: d.expected.map(|e| EventRef {
                kind: e.kind,
                unit: e.unit,
            }),
            actual: d.actual.map(|a| EventRef {
                kind: a.kind,
                unit: a.unit,
            }),
        }),
    }
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

/// Divergence-detail fields shared between single and multi-baseline JSON
/// reports. `ObservedOutcome` and `ObservedEventKind` serialize through
/// their `Serialize` derives so the wire format matches the embedded
/// `Observation`'s `outcome` / event `kind` values.
#[derive(Serialize)]
struct CompareReportBody<'a> {
    classification: &'static str,
    mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome_mismatch: Option<OutcomePair>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_divergence: Option<MemoryDiv<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_divergence: Option<EventDiv>,
}

#[derive(Serialize)]
struct JsonReport<'a> {
    #[serde(flatten)]
    body: CompareReportBody<'a>,
    expected: &'a Observation,
    actual: &'a Observation,
}

#[derive(Serialize)]
struct OutcomePair {
    expected: ObservedOutcome,
    actual: ObservedOutcome,
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
    kind: ObservedEventKind,
    unit: u64,
}

#[derive(Serialize)]
struct MultiJsonReport<'a> {
    classification: &'static str,
    mode: &'static str,
    baseline_count: usize,
    oracle_settled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    oracle_divergence: Option<CompareReportBody<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cellgov_result: Option<CompareReportBody<'a>>,
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

    #[test]
    fn multi_json_settled_includes_cellgov_result_details() {
        let result = MultiCompareResult {
            classification: Classification::Divergence,
            mode: CompareMode::Strict,
            oracle_divergence: None,
            cellgov_result: Some(CompareResult {
                classification: Classification::Divergence,
                mode: CompareMode::Strict,
                outcome_mismatch: Some((ObservedOutcome::Completed, ObservedOutcome::Fault)),
                memory_divergence: Some(MemoryDivergence {
                    region: "result".into(),
                    offset: 7,
                    expected: 0xAA,
                    actual: 0xBB,
                }),
                event_divergence: None,
            }),
        };
        let a = obs(ObservedOutcome::Completed);
        let json = format_multi_json(&result, std::slice::from_ref(&a), &a).expect("json");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        let cg = &parsed["cellgov_result"];
        assert_eq!(cg["classification"], "divergence");
        assert_eq!(cg["outcome_mismatch"]["expected"], "Completed");
        assert_eq!(cg["outcome_mismatch"]["actual"], "Fault");
        assert_eq!(cg["memory_divergence"]["region"], "result");
        assert_eq!(cg["memory_divergence"]["offset"], 7);
        assert!(parsed.get("oracle_divergence").is_none());
    }

    #[test]
    fn multi_json_unsettled_includes_oracle_divergence_details() {
        let result = MultiCompareResult {
            classification: Classification::UnsettledOracle,
            mode: CompareMode::Events,
            oracle_divergence: Some(CompareResult {
                classification: Classification::Divergence,
                mode: CompareMode::Events,
                outcome_mismatch: None,
                memory_divergence: None,
                event_divergence: Some(EventDivergence {
                    index: 3,
                    expected: Some(ObservedEvent {
                        kind: ObservedEventKind::DmaComplete,
                        unit: 1,
                        sequence: 3,
                    }),
                    actual: None,
                }),
            }),
            cellgov_result: None,
        };
        let a = obs(ObservedOutcome::Completed);
        let b = obs(ObservedOutcome::Fault);
        let json =
            format_multi_json(&result, &[a, b], &obs(ObservedOutcome::Completed)).expect("json");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        let od = &parsed["oracle_divergence"];
        assert_eq!(od["classification"], "divergence");
        assert_eq!(od["event_divergence"]["index"], 3);
        assert_eq!(od["event_divergence"]["expected"]["kind"], "DmaComplete");
        assert_eq!(od["event_divergence"]["expected"]["unit"], 1);
        assert!(od["event_divergence"].get("actual").is_none());
        assert!(parsed.get("cellgov_result").is_none());
    }

    #[test]
    fn json_outcome_serialization_matches_observation_schema() {
        let result = CompareResult {
            classification: Classification::Divergence,
            mode: CompareMode::Strict,
            outcome_mismatch: Some((ObservedOutcome::Completed, ObservedOutcome::Fault)),
            memory_divergence: None,
            event_divergence: None,
        };
        let a = obs(ObservedOutcome::Completed);
        let b = obs(ObservedOutcome::Fault);
        let json = format_json(&result, &a, &b).expect("json");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(parsed["outcome_mismatch"]["expected"], "Completed");
        assert_eq!(parsed["expected"]["outcome"], "Completed");
        assert_eq!(parsed["outcome_mismatch"]["actual"], "Fault");
        assert_eq!(parsed["actual"]["outcome"], "Fault");
    }
}
