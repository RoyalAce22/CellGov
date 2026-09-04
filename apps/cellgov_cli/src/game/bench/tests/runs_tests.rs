use std::time::Duration;

use super::super::anchor::AnchorVerdict;
use super::super::test_fixtures::{reporting, set_of, strict};
use super::super::throughput::{throughput_verdict, ThroughputVerdict};
use super::super::types::BenchGate;
use super::*;

#[test]
fn strict_perf_fails_on_either_way_of_reaching_no_verdict() {
    let inconclusive = throughput_verdict(&set_of(&[
        Duration::from_millis(100),
        Duration::from_millis(200),
    ]));
    for verdict in [inconclusive, ThroughputVerdict::Unmeasurable] {
        assert_eq!(
            classify_runs(&[], &AnchorVerdict::Skipped, verdict, strict()),
            BenchGate::SpreadExceeded,
            "{verdict:?}"
        );
    }
}

#[test]
fn strict_perf_passes_a_measured_set() {
    let verdict = throughput_verdict(&set_of(&[
        Duration::from_millis(100),
        Duration::from_millis(102),
    ]));
    assert_eq!(
        classify_runs(&[], &AnchorVerdict::Skipped, verdict, strict()),
        BenchGate::Pass
    );
}

#[test]
fn a_determinism_break_outranks_a_measured_throughput() {
    let runs = set_of(&[Duration::from_millis(100); 2]);
    assert_eq!(
        classify_runs(
            &["a witness moved".to_string()],
            &AnchorVerdict::Match,
            throughput_verdict(&runs),
            strict()
        ),
        BenchGate::DeterminismBreak
    );
}

#[test]
fn anchor_drift_outranks_an_inconclusive_throughput() {
    let runs = set_of(&[Duration::from_millis(100), Duration::from_millis(200)]);
    let anchor = AnchorVerdict::Drift(vec!["host_invariant_breaks moved".to_string()]);
    assert_eq!(
        classify_runs(&[], &anchor, throughput_verdict(&runs), strict()),
        BenchGate::AnchorDrift,
        "a contended host must not mask a real anchor regression",
    );
}

#[test]
fn a_determinism_break_outranks_anchor_drift() {
    let runs = set_of(&[Duration::from_millis(100); 2]);
    let anchor = AnchorVerdict::Drift(vec!["host_invariant_breaks moved".to_string()]);
    assert_eq!(
        classify_runs(
            &["run 1 retired 10 steps, run 2 retired 11".to_string()],
            &anchor,
            throughput_verdict(&runs),
            reporting()
        ),
        BenchGate::DeterminismBreak,
    );
}

#[test]
fn a_skipped_anchor_check_cannot_produce_anchor_drift() {
    let throughput = throughput_verdict(&set_of(&[Duration::from_millis(100); 2]));
    for anchor in [
        AnchorVerdict::Skipped,
        AnchorVerdict::NotRecorded("fw 4.93 x base".into()),
        AnchorVerdict::NotComparable(vec!["retargeted".to_string()]),
    ] {
        assert_eq!(
            classify_runs(&[], &anchor, throughput, reporting()),
            BenchGate::Pass,
            "{anchor:?}"
        );
    }
}
