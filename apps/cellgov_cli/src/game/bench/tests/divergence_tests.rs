use std::time::Duration;

use cellgov_compare::BootOutcome;
use cellgov_time::Budget;

use super::super::anchor::AnchorVerdict;
use super::super::runs::classify_runs;
use super::super::test_fixtures::{bench_manifest, bench_options, reporting, set_of, test_cell};
use super::super::throughput::throughput_verdict;
use super::super::types::BenchGate;
use super::*;

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
        classify_runs(
            &[],
            &AnchorVerdict::Skipped,
            throughput_verdict(&runs),
            reporting()
        ),
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
        classify_runs(
            &failures,
            &AnchorVerdict::Skipped,
            throughput_verdict(&runs),
            reporting()
        ),
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
        classify_runs(
            &failures,
            &AnchorVerdict::Match,
            throughput_verdict(&runs),
            reporting()
        ),
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

#[test]
fn a_long_boot_names_the_localization_commands_instead_of_running_them() {
    let title = bench_manifest(None);
    let cell = test_cell();
    let opts = bench_options(&title, Some(&cell), &[]);
    let mut runs = set_of(&[Duration::from_millis(100)]);
    runs[0].steps = LOCALIZE_MAX_STEPS + 1;
    let lines = locate_divergence(opts, &runs).expect("localization does not interrupt");
    assert!(lines[0].contains("not run automatically"), "got {lines:?}");
    assert!(
        lines.iter().any(|l| l.contains("--save-state-trace"))
            && lines.iter().any(|l| l.contains("diff diverge")),
        "the report must name every command the operator has to run: {lines:?}"
    );
}

#[test]
fn the_localization_cap_reads_the_longest_run_not_the_first() {
    let title = bench_manifest(None);
    let cell = test_cell();
    let opts = bench_options(&title, Some(&cell), &[]);
    let mut runs = set_of(&[Duration::from_millis(100); 2]);
    runs[0].steps = 10;
    runs[1].steps = LOCALIZE_MAX_STEPS + 1;
    let lines = locate_divergence(opts, &runs).expect("localization does not interrupt");
    assert!(lines[0].contains("not run automatically"), "got {lines:?}");
    assert!(
        lines[0].contains(&(LOCALIZE_MAX_STEPS + 1).to_string()),
        "the report must name the run that sets the cost: {lines:?}"
    );
}

#[test]
fn a_corrupt_trace_report_names_the_side_and_its_decode_failure() {
    let line = format_diverge(&cellgov_compare::DivergeReport::CorruptTrace {
        common_count: 12,
        a_error: None,
        b_error: Some(cellgov_compare::TraceDecodeError {
            index: 4,
            offset: 96,
            source: cellgov_trace::DecodeError::UnknownTag(0xee),
        }),
    });
    assert!(line.contains("a: ok"), "got {line}");
    assert!(
        line.contains("record 4") && line.contains("unknown record tag 0xee"),
        "got {line}"
    );
}

#[test]
fn a_failing_traced_re_run_reports_its_stderr_tail_in_order() {
    let stderr: String = (0..20).map(|i| format!("line {i}\n")).collect();
    let tail = stderr_tail(stderr.as_bytes());
    assert_eq!(tail.len(), 8);
    assert_eq!(tail.first().map(String::as_str), Some("  line 12"));
    assert_eq!(tail.last().map(String::as_str), Some("  line 19"));
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
        "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=73
"
        .to_string(),
        "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=76
"
        .to_string(),
    ];
    let runs = set_of(&[Duration::from_millis(100); 2]);
    let failures = determinism_disagreements(&runs, &streams);
    assert_eq!(
        failures,
        vec!["witness host_invariant_breaks: run 1 73 != run 2 76".to_string()]
    );
    assert_eq!(
        classify_runs(
            &failures,
            &AnchorVerdict::Match,
            throughput_verdict(&runs),
            reporting()
        ),
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
        "BENCH_DCBZ_WITNESS: count=0
"
        .to_string(),
        "BENCH_DCBZ_WITNESS: count=0
"
        .to_string(),
        "BENCH_DCBZ_WITNESS: count=4
"
        .to_string(),
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
    let good = "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=73
";
    let bad = "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=lots
";
    assert!(witness_disagreements("run 1", good, "run 2", bad)[0]
        .starts_with("run 2 witness line did not parse"));
    assert!(witness_disagreements("run 1", bad, "run 2", good)[0]
        .starts_with("run 1 witness line did not parse"));
}
