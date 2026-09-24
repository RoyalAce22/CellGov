use std::time::Duration;

use cellgov_time::Budget;

use super::super::test_fixtures::set_of;
use super::*;
use crate::runner_cellgov::BootOutcome;

#[test]
fn a_set_whose_runs_reproduce_each_other_passes() {
    let runs = set_of(&[Duration::from_millis(100), Duration::from_millis(102)]);
    // `parse_witness_lines("")` succeeds with an empty map, so two
    // blank streams would agree over nothing and the witness half of
    // this case could never fail.
    let witnesses = "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=73\n\
                     BENCH_ATOMIC_WITNESS: ldarx=100 stdcx=0 lwarx=0 stwcx=0\n";
    let streams = [witnesses.to_string(), witnesses.to_string()];
    assert!(determinism_disagreements(&runs, &streams).is_empty());
    assert_eq!(
        classify_runs(&[], &AnchorVerdict::Skipped, true, false),
        BenchGate::Pass
    );
}

#[test]
fn a_moved_step_count_between_runs_is_a_determinism_break() {
    let mut runs = set_of(&[Duration::from_millis(100); 2]);
    runs[1].steps += 1;
    let failures = determinism_disagreements(&runs, &["".to_string(), "".to_string()]);
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("run 1 retired 10 steps") && failures[0].contains("run 2 retired 11"),
        "got {failures:?}"
    );
    assert_eq!(
        classify_runs(&failures, &AnchorVerdict::Skipped, true, false),
        BenchGate::DeterminismBreak
    );
}

#[test]
fn a_moved_outcome_between_runs_is_a_determinism_break() {
    let mut runs = set_of(&[Duration::from_millis(100); 2]);
    runs[1].outcome = BootOutcome::MaxSteps;
    let failures = determinism_disagreements(&runs, &["".to_string(), "".to_string()]);
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("ProcessExit") && failures[0].contains("MaxSteps"),
        "got {failures:?}"
    );
}

#[test]
fn a_moved_budget_between_runs_is_a_determinism_break() {
    let mut runs = set_of(&[Duration::from_millis(100); 2]);
    runs[1].budget = Budget::new(512);
    let failures = determinism_disagreements(&runs, &["".to_string(), "".to_string()]);
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("run 1 ran at budget 256")
            && failures[0].contains("run 2 ran at budget 512"),
        "got {failures:?}"
    );
    assert_eq!(
        classify_runs(&failures, &AnchorVerdict::Match, true, false),
        BenchGate::DeterminismBreak
    );
}

#[test]
fn every_run_is_compared_against_the_first() {
    let mut runs = set_of(&[Duration::from_millis(100); 4]);
    runs[2].steps += 1;
    let streams = vec![String::new(); 4];
    let failures = determinism_disagreements(&runs, &streams);
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(failures[0].contains("run 3 retired 11"), "got {failures:?}");
}

/// A run whose stream is missing cannot show its witnesses agree, so it
/// must not pass for one that did.
#[test]
fn a_run_with_no_stream_is_a_disagreement() {
    let runs = set_of(&[Duration::from_millis(100); 3]);
    let streams = vec![String::new(); 2];
    assert_eq!(
        determinism_disagreements(&runs, &streams),
        vec!["run 3 carries no stream to compare witnesses against".to_string()]
    );
}

#[test]
fn identical_witness_streams_disagree_nowhere() {
    let stream = "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=73
                  BENCH_ATOMIC_WITNESS: ldarx=100 stdcx=0 lwarx=0 stwcx=0
";
    assert!(witness_disagreements("run 1", stream, "run 2", stream).is_empty());
}

/// The steps/outcome comparison cannot see this, and the anchor check
/// reads run 1 alone, so without the witness check a counter that
/// moves between runs passes the gate.
#[test]
fn a_witness_that_moved_between_runs_is_a_determinism_break() {
    let streams = [
        "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=73\n".to_string(),
        "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=76\n".to_string(),
    ];
    let runs = set_of(&[Duration::from_millis(100); 2]);
    let failures = determinism_disagreements(&runs, &streams);
    assert_eq!(
        failures,
        vec!["witness host_invariant_breaks: run 1 73 != run 2 76".to_string()]
    );
    assert_eq!(
        classify_runs(&failures, &AnchorVerdict::Match, true, false),
        BenchGate::DeterminismBreak,
        "agreeing steps, outcome and anchor must not outvote a moving witness",
    );
}

#[test]
fn a_witness_line_only_one_run_emitted_is_a_disagreement() {
    let r1 = "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=73
              BENCH_DCBZ_WITNESS: count=0
";
    let r2 = "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=73
";
    let failures = witness_disagreements("run 1", r1, "run 2", r2);
    assert_eq!(
        failures,
        vec![
            "witness line BENCH_DCBZ_WITNESS: appeared in run 1 only".to_string(),
            "witness dcbz: run 1 0, absent from run 2".to_string(),
        ]
    );
}

#[test]
fn a_disagreement_is_labelled_with_the_run_it_came_from() {
    let streams = vec![
        "BENCH_DCBZ_WITNESS: count=0\n".to_string(),
        "BENCH_DCBZ_WITNESS: count=0\n".to_string(),
        "BENCH_DCBZ_WITNESS: count=4\n".to_string(),
    ];
    let runs = set_of(&[Duration::from_millis(100); 3]);
    let failures = determinism_disagreements(&runs, &streams);
    assert_eq!(
        failures,
        vec!["witness dcbz: run 1 0 != run 3 4".to_string()]
    );
}

#[test]
fn a_malformed_witness_line_in_either_run_is_a_disagreement() {
    let good = "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=73\n";
    let bad = "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=lots\n";
    assert!(witness_disagreements("run 1", good, "run 2", bad)[0]
        .starts_with("run 2 witness line did not parse"));
    assert!(witness_disagreements("run 1", bad, "run 2", good)[0]
        .starts_with("run 1 witness line did not parse"));
}

#[test]
fn a_strict_throughput_policy_fails_a_set_with_no_throughput_claim() {
    assert_eq!(
        classify_runs(&[], &AnchorVerdict::Skipped, false, true),
        BenchGate::SpreadExceeded
    );
    assert_eq!(
        classify_runs(&[], &AnchorVerdict::Skipped, true, true),
        BenchGate::Pass
    );
    assert_eq!(
        classify_runs(&[], &AnchorVerdict::Skipped, false, false),
        BenchGate::Pass,
        "without the strict policy a busy host fails nothing"
    );
}

#[test]
fn anchor_drift_outranks_a_missing_throughput_claim() {
    let anchor = AnchorVerdict::Drift(vec!["host_invariant_breaks moved".to_string()]);
    assert_eq!(
        classify_runs(&[], &anchor, false, true),
        BenchGate::AnchorDrift,
        "a contended host must not mask a real anchor regression",
    );
}

#[test]
fn a_determinism_break_outranks_anchor_drift() {
    let anchor = AnchorVerdict::Drift(vec!["host_invariant_breaks moved".to_string()]);
    assert_eq!(
        classify_runs(
            &["run 1 retired 10 steps, run 2 retired 11".to_string()],
            &anchor,
            true,
            false
        ),
        BenchGate::DeterminismBreak,
    );
}

#[test]
fn a_skipped_anchor_check_cannot_produce_anchor_drift() {
    for anchor in [
        AnchorVerdict::Skipped,
        AnchorVerdict::NotRecorded("fw 4.93 x base".into()),
        AnchorVerdict::NotComparable(vec!["retargeted".to_string()]),
    ] {
        assert_eq!(
            classify_runs(&[], &anchor, true, false),
            BenchGate::Pass,
            "{anchor:?}"
        );
    }
}
