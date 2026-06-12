//! JSON renderers for single and multi-baseline reports.

use serde::Serialize;

use crate::compare::{CompareResult, MultiCompareResult};
use crate::observation::{Observation, ObservedEventKind, ObservedOutcome};

use super::labels::{classification_slug, mode_str};

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
#[path = "tests/json_tests.rs"]
mod tests;
