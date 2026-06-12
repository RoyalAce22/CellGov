//! JSON report shape and field presence for single and multi-baseline compare results.

use super::*;
use crate::compare::{
    Classification, CompareMode, CompareResult, EventDivergence, MemoryDivergence,
    MultiCompareResult,
};
use crate::observation::{NamedMemoryRegion, ObservedEvent};
use crate::test_support::obs as obs_full;

fn obs(outcome: ObservedOutcome) -> Observation {
    obs_full(outcome, vec![], vec![])
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
    let json = format_multi_json(&result, &[a, b], &obs(ObservedOutcome::Completed)).expect("json");
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
    let json = format_multi_json(&result, &[a, b], &obs(ObservedOutcome::Completed)).expect("json");
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
