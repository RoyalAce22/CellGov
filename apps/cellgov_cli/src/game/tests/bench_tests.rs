//! Bench-result line parsing, the throughput verdict, and the anchor
//! comparison the run-set gate runs.

use std::collections::BTreeSet;
use std::time::Duration;

use super::*;

/// A run of `wall` that agrees with every other run this helper
/// builds, so a case that varies only the wall isolates the throughput
/// half.
fn run_of(run_index: usize, wall: Duration) -> BenchBootResult {
    BenchBootResult {
        run_index,
        steps: 10,
        wall,
        budget: Budget::new(256),
        outcome: BootOutcome::ProcessExit,
    }
}

/// A set whose walls are `walls`, indexed in order.
fn set_of(walls: &[Duration]) -> Vec<BenchBootResult> {
    walls
        .iter()
        .enumerate()
        .map(|(i, w)| run_of(i, *w))
        .collect()
}

fn reporting() -> ThroughputPolicy {
    ThroughputPolicy {
        runs: 3,
        strict: false,
    }
}

fn strict() -> ThroughputPolicy {
    ThroughputPolicy {
        runs: 3,
        strict: true,
    }
}

#[test]
fn identical_walls_spread_nowhere() {
    let set = set_of(&[Duration::from_millis(1000); 3]);
    assert_eq!(
        throughput_verdict(&set),
        ThroughputVerdict::Measured {
            min: Duration::from_millis(1000),
            spread_pct: 0.0,
        }
    );
}

#[test]
fn the_estimate_is_the_fastest_run_not_the_mean() {
    let set = set_of(&[
        Duration::from_millis(100),
        Duration::from_millis(180),
        Duration::from_millis(140),
    ]);
    let ThroughputVerdict::Inconclusive { min, spread_pct } = throughput_verdict(&set) else {
        panic!("an 80% spread is above the ceiling");
    };
    assert_eq!(min, Duration::from_millis(100));
    assert!((spread_pct - 80.0).abs() < 0.0001, "got {spread_pct}");
}

#[test]
fn the_spread_is_relative_to_the_fastest_run() {
    let set = set_of(&[Duration::from_millis(100), Duration::from_millis(104)]);
    let ThroughputVerdict::Measured { spread_pct, .. } = throughput_verdict(&set) else {
        panic!("4% is inside the ceiling");
    };
    assert!((spread_pct - 4.0).abs() < 0.0001, "got {spread_pct}");
}

/// `100.0 * (21.0 - 20.0) / 20.0` is exact in binary floating point,
/// so these two walls land on the ceiling and the assertion needs no
/// tolerance.
#[test]
fn a_spread_exactly_at_the_ceiling_is_measured() {
    let set = set_of(&[Duration::from_secs(20), Duration::from_secs(21)]);
    let ThroughputVerdict::Measured { spread_pct, .. } = throughput_verdict(&set) else {
        panic!("a spread equal to the ceiling is inside it");
    };
    assert_eq!(spread_pct, BENCH_SPREAD_CEILING_PCT);
}

#[test]
fn a_spread_just_past_the_ceiling_is_inconclusive() {
    let set = set_of(&[Duration::from_secs(20), Duration::from_millis(21_001)]);
    assert!(matches!(
        throughput_verdict(&set),
        ThroughputVerdict::Inconclusive { .. }
    ));
}

#[test]
fn a_set_of_one_run_makes_no_spread_claim() {
    let policy = ThroughputPolicy {
        runs: 1,
        strict: false,
    };
    let verdict = throughput_verdict(&set_of(&[Duration::from_millis(100)]));
    let line = throughput_line(verdict, policy);
    assert!(line.contains("NOT CHECKED"), "got {line}");
    assert!(!line.contains('%'), "got {line}");
}

#[test]
fn a_set_of_several_runs_reports_the_spread_it_measured() {
    let policy = ThroughputPolicy {
        runs: 2,
        strict: false,
    };
    let verdict = throughput_verdict(&set_of(&[
        Duration::from_millis(100),
        Duration::from_millis(102),
    ]));
    let line = throughput_line(verdict, policy);
    assert!(line.contains("spread 2.00%"), "got {line}");
}

#[test]
fn a_zero_wall_leaves_the_throughput_unmeasurable() {
    let set = set_of(&[Duration::ZERO, Duration::from_millis(100)]);
    assert_eq!(throughput_verdict(&set), ThroughputVerdict::Unmeasurable);
}

#[test]
fn a_spread_above_the_ceiling_reports_and_does_not_fail() {
    let set = set_of(&[Duration::from_millis(100), Duration::from_millis(200)]);
    let verdict = throughput_verdict(&set);
    assert!(matches!(verdict, ThroughputVerdict::Inconclusive { .. }));
    assert_eq!(
        classify_runs(&[], &AnchorVerdict::Skipped, verdict, reporting()),
        BenchGate::Pass
    );
}

#[test]
fn an_unmeasurable_wall_reports_and_does_not_fail() {
    assert_eq!(
        classify_runs(
            &[],
            &AnchorVerdict::Skipped,
            ThroughputVerdict::Unmeasurable,
            reporting()
        ),
        BenchGate::Pass
    );
}

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
fn parse_bench_result_round_trips_every_boot_outcome() {
    let variants = [
        BootOutcome::ProcessExit,
        BootOutcome::Fault,
        BootOutcome::MaxSteps,
        BootOutcome::RsxWriteCheckpoint,
        BootOutcome::PcReached(0x10381ce8),
        BootOutcome::TimeOverflow,
    ];
    for v in variants {
        let line = format!(
            "BENCH_RESULT run_index=0 steps=1 wall_ns=1 steps_per_sec=1000000000 budget=256 outcome={v}\n"
        );
        let r = parse_bench_result(&line)
            .unwrap_or_else(|e| panic!("round-trip parse failed for {v:?}: {e}"));
        assert_eq!(r.outcome, v, "round-trip mismatch for {v:?}");
    }
}

#[test]
fn parse_bench_result_extracts_fields() {
    let stdout = "some preamble\nBENCH_RESULT run_index=2 steps=1402388 wall_ns=323000000 steps_per_sec=4341759 budget=256 outcome=ProcessExit\ntrailing noise\n";
    let r = parse_bench_result(stdout).expect("parses");
    assert_eq!(r.run_index, 2);
    assert_eq!(r.steps, 1402388);
    assert_eq!(r.wall.as_millis(), 323);
    assert_eq!(r.outcome, BootOutcome::ProcessExit);
}

#[test]
fn the_result_line_round_trips_the_run_index() {
    for index in [0usize, 1, 7] {
        let r = BenchBootResult {
            run_index: index,
            steps: 12345,
            wall: Duration::from_millis(3),
            budget: Budget::new(256),
            outcome: BootOutcome::MaxSteps,
        };
        let parsed = parse_bench_result(&format_bench_result(&r)).expect("parses");
        assert_eq!(parsed.run_index, index);
    }
}

#[test]
fn parse_bench_result_errors_on_missing_run_index() {
    let stdout = "BENCH_RESULT steps=1 wall_ns=1 steps_per_sec=1 budget=256 outcome=ProcessExit\n";
    assert_eq!(
        parse_bench_result(stdout).unwrap_err(),
        ParseBenchError::MissingRunIndex
    );
}

#[test]
fn parse_bench_result_errors_on_malformed_run_index() {
    let stdout =
        "BENCH_RESULT run_index=last steps=1 wall_ns=1 steps_per_sec=1 budget=256 outcome=ProcessExit\n";
    match parse_bench_result(stdout).unwrap_err() {
        ParseBenchError::MalformedRunIndex(s) => assert_eq!(s, "last"),
        other => panic!("expected MalformedRunIndex, got {other:?}"),
    }
}

#[test]
fn the_result_line_round_trips_the_wall_exactly() {
    for ns in [1u64, 750, 999_999, 1_000_001, 1_234_567_891, 3_037_000_123] {
        let r = BenchBootResult {
            run_index: 0,
            steps: 12345,
            wall: Duration::from_nanos(ns),
            budget: Budget::new(256),
            outcome: BootOutcome::MaxSteps,
        };
        let parsed = parse_bench_result(&format!("{}\n", format_bench_result(&r))).expect("parses");
        assert_eq!(parsed.wall, r.wall, "wall_ns={ns}");
        assert_eq!(parsed.steps, r.steps);
        assert_eq!(parsed.outcome, r.outcome);
    }
}

#[test]
fn a_sub_microsecond_wall_is_measurable_after_transport() {
    let r = BenchBootResult {
        run_index: 0,
        steps: 3,
        wall: Duration::from_nanos(400),
        budget: Budget::new(256),
        outcome: BootOutcome::ProcessExit,
    };
    let parsed = parse_bench_result(&format_bench_result(&r)).expect("parses");
    assert_eq!(parsed.wall, Duration::from_nanos(400));
    assert!(
        throughput_verdict(&[parsed]).is_measured(),
        "a 400 ns run must not read as an unmeasurable zero wall"
    );
}

#[test]
fn the_printed_steps_per_sec_agrees_with_the_recomputed_one_to_rounding() {
    let r = BenchBootResult {
        run_index: 0,
        steps: 390_435,
        wall: Duration::from_nanos(3_038_513_400),
        budget: Budget::new(256),
        outcome: BootOutcome::MaxSteps,
    };
    let line = format_bench_result(&r);
    let reported: f64 = line
        .split_whitespace()
        .find_map(|t| t.strip_prefix("steps_per_sec="))
        .expect("steps_per_sec token")
        .parse()
        .expect("numeric");
    let parsed = parse_bench_result(&line).expect("parses");
    assert!(
        (reported - parsed.steps_per_sec()).abs() <= 0.5,
        "reported {reported} vs recomputed {}",
        parsed.steps_per_sec()
    );
}

#[test]
fn a_wall_beyond_u64_nanoseconds_is_malformed_not_clamped() {
    let stdout =
        "BENCH_RESULT run_index=0 steps=1 wall_ns=99999999999999999999999 steps_per_sec=1 budget=256 outcome=ProcessExit\n";
    match parse_bench_result(stdout).unwrap_err() {
        ParseBenchError::MalformedWallNs(s) => assert_eq!(s, "99999999999999999999999"),
        other => panic!("expected MalformedWallNs, got {other:?}"),
    }
}

#[test]
fn parse_bench_result_errors_on_missing_line() {
    let stdout = "just some noise\nbut no result line\n";
    assert_eq!(
        parse_bench_result(stdout).unwrap_err(),
        ParseBenchError::NoResultLine
    );
}

#[test]
fn parse_bench_result_errors_on_duplicate_line() {
    let stdout = "BENCH_RESULT run_index=0 steps=1 wall_ns=1 steps_per_sec=1 budget=256 outcome=ProcessExit\n\
                  BENCH_RESULT run_index=1 steps=2 wall_ns=2 steps_per_sec=1 budget=256 outcome=ProcessExit\n";
    assert_eq!(
        parse_bench_result(stdout).unwrap_err(),
        ParseBenchError::DuplicateResultLine
    );
}

#[test]
fn parse_bench_result_errors_on_unknown_outcome() {
    let stdout =
        "BENCH_RESULT run_index=0 steps=1 wall_ns=1 steps_per_sec=1 budget=256 outcome=WhoKnows\n";
    match parse_bench_result(stdout).unwrap_err() {
        ParseBenchError::UnparseableOutcome { token, source: _ } => {
            assert_eq!(token, "WhoKnows");
        }
        other => panic!("expected UnparseableOutcome, got {other:?}"),
    }
}

#[test]
fn parse_bench_result_errors_on_malformed_steps() {
    let stdout =
        "BENCH_RESULT run_index=0 steps=abc wall_ns=1 steps_per_sec=1 budget=256 outcome=ProcessExit\n";
    match parse_bench_result(stdout).unwrap_err() {
        ParseBenchError::MalformedSteps(s) => assert_eq!(s, "abc"),
        other => panic!("expected MalformedSteps, got {other:?}"),
    }
}

#[test]
fn parse_bench_result_errors_on_missing_steps() {
    let stdout =
        "BENCH_RESULT run_index=0 wall_ns=1 steps_per_sec=1 budget=256 outcome=ProcessExit\n";
    assert_eq!(
        parse_bench_result(stdout).unwrap_err(),
        ParseBenchError::MissingSteps
    );
}

#[test]
fn parse_bench_result_errors_on_malformed_wall_ns() {
    let stdout =
        "BENCH_RESULT run_index=0 steps=1 wall_ns=xyz steps_per_sec=1 budget=256 outcome=ProcessExit\n";
    match parse_bench_result(stdout).unwrap_err() {
        ParseBenchError::MalformedWallNs(s) => assert_eq!(s, "xyz"),
        other => panic!("expected MalformedWallNs, got {other:?}"),
    }
}

#[test]
fn parse_bench_result_errors_on_missing_wall_ns() {
    let stdout =
        "BENCH_RESULT run_index=0 steps=1 steps_per_sec=1 budget=256 outcome=ProcessExit\n";
    assert_eq!(
        parse_bench_result(stdout).unwrap_err(),
        ParseBenchError::MissingWallNs
    );
}

#[test]
fn parse_bench_result_errors_on_missing_outcome() {
    let stdout = "BENCH_RESULT run_index=0 steps=1 wall_ns=1 steps_per_sec=1 budget=256\n";
    assert_eq!(
        parse_bench_result(stdout).unwrap_err(),
        ParseBenchError::MissingOutcome
    );
}

#[test]
fn parse_bench_result_errors_on_missing_budget() {
    let stdout = "BENCH_RESULT run_index=0 steps=1 wall_ns=1 steps_per_sec=1 outcome=ProcessExit\n";
    assert_eq!(
        parse_bench_result(stdout).unwrap_err(),
        ParseBenchError::MissingBudget
    );
}

#[test]
fn parse_bench_result_errors_on_malformed_budget() {
    let stdout = "BENCH_RESULT run_index=0 steps=1 wall_ns=1 steps_per_sec=1 budget=lots outcome=ProcessExit\n";
    match parse_bench_result(stdout).unwrap_err() {
        ParseBenchError::MalformedBudget(s) => assert_eq!(s, "lots"),
        other => panic!("expected MalformedBudget, got {other:?}"),
    }
}

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
    let lines = locate_divergence(opts, &runs);
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
    let lines = locate_divergence(opts, &runs);
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

/// The stop condition [`anchor_fixture`] records.
const TEST_CHECKPOINT: manifest::CheckpointTrigger = manifest::CheckpointTrigger::ProcessExit;

/// A run that reproduces [`anchor_fixture`] exactly, as the set hands
/// it to the anchor check.
fn measured_run(stderr: &str) -> MeasuredRun<'_> {
    MeasuredRun {
        checkpoint: TEST_CHECKPOINT,
        steps: 1,
        budget: Budget::new(256),
        outcome: "MaxSteps".to_string(),
        stderr,
    }
}

/// The cell [`anchor_fixture`] is filed under.
fn test_cell() -> CellKey {
    CellKey {
        fw: "4.93".to_string(),
        game_ver: Some("base".to_string()),
    }
}

/// The triple [`anchor_fixture`] embeds.
fn test_identity() -> RunIdentity {
    anchor_fixture(0).identity
}

/// Mirrors a committed `boot_summary.json`, so the fixture format and
/// the comparison are exercised through the deserializer production
/// uses.
fn anchor_fixture(breaks: u64) -> BootSummary {
    serde_json::from_str(&format!(
        r#"{{
          "checkpoint": {{ "kind": "process_exit" }},
          "outcome": "MaxSteps",
          "steps": 390099,
          "budget": 256,
          "host_invariant_breaks": {breaks},
          "witnesses": {{
            "host_invariant_breaks": {{ "value": {breaks}, "class": "exact" }},
            "ldarx": {{ "value": 100, "class": "at-least" }},
            "stdcx": {{ "value": 0, "class": "at-least" }},
            "lwarx": {{ "value": 0, "class": "at-least" }},
            "stwcx": {{ "value": 0, "class": "at-least" }}
          }},
          "firmware": {{
            "version": "4.93",
            "image_version": "0x0000000000010b94",
            "pup_sha256": "00"
          }},
          "game": {{
            "title_id": "CG_TEST",
            "version": "base",
            "app_ver": "01.00"
          }}
        }}"#
    ))
    .expect("anchor fixture parses")
}

fn observed_stderr(breaks: u64, ldarx: u64) -> ParsedWitnesses {
    parse_witness_lines(&format!(
        "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count={breaks}\n\
         BENCH_ATOMIC_WITNESS: ldarx={ldarx} stdcx=0 lwarx=0 stwcx=0\n"
    ))
    .expect("synthetic witness lines parse")
}

#[test]
fn a_run_matching_its_anchor_reports_no_disagreements() {
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert!(
        failures.is_empty(),
        "expected no failures, got {failures:?}"
    );
}

#[test]
fn an_exact_witness_that_moved_is_reported() {
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(77, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("host_invariant_breaks") && failures[0].contains("77"),
        "failure must name the witness and the observed value: {}",
        failures[0]
    );
}

#[test]
fn an_at_least_witness_above_its_baseline_is_not_a_disagreement() {
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 9_999),
    );
    assert!(failures.is_empty(), "got {failures:?}");
}

#[test]
fn a_moved_step_count_is_reported() {
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &test_identity(),
        TEST_CHECKPOINT,
        390100,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert!(
        failures.iter().any(|f| f.contains("steps 390100")),
        "got {failures:?}"
    );
}

#[test]
fn a_changed_outcome_is_reported() {
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "ProcessExit",
        &observed_stderr(73, 100),
    );
    assert!(
        failures.iter().any(|f| f.contains("outcome ProcessExit")),
        "got {failures:?}"
    );
}

/// A stop condition the run never reaches leaves the step count, the
/// outcome and every witness intact, so nothing else in the comparison
/// sees a moved checkpoint.
#[test]
fn a_run_taken_at_another_checkpoint_is_reported() {
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &test_identity(),
        manifest::CheckpointTrigger::Pc(0x1_0000),
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("Pc=0x10000") && failures[0].contains("ProcessExit"),
        "the failure must name both stop conditions: {}",
        failures[0]
    );
}

/// A moved budget retires a different trajectory under a step count
/// that did not move, and the recorded witnesses are at-least bounds
/// that do not catch it.
#[test]
fn a_run_at_another_budget_is_reported() {
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        Budget::new(512),
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("budget 512") && failures[0].contains("256"),
        "got {failures:?}"
    );
}

/// The steps and the witnesses come out of the measuring child, so the
/// triple must come out of that same stream.
#[test]
fn a_measured_run_that_named_no_triple_is_a_disagreement() {
    let root = crate::paths::workspace_root();
    let verdict = check_anchor_under(
        &root,
        "NPUA80001",
        &test_cell(),
        &measured_run("BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=0\n"),
    );
    let AnchorVerdict::Drift(failures) = verdict else {
        panic!("expected Drift, got {verdict:?}");
    };
    assert!(
        failures
            .iter()
            .any(|f| f.contains(RUN_IDENTITY_SENTINEL) && f.contains("no")),
        "got {failures:?}"
    );
}

/// A file copied from another cell reproduces every witness of its own
/// run, so only the embedded triple names the wrong cell.
#[test]
fn an_anchor_measured_against_another_firmware_reports_the_triple() {
    let mut ran = test_identity();
    ran.firmware.as_mut().expect("firmware half").version = "3.55".to_string();
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &ran,
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("recorded 4.93") && failures[0].contains("ran 3.55"),
        "got {failures:?}"
    );
}

#[test]
fn an_anchor_measured_against_another_title_version_reports_the_triple() {
    let mut ran = test_identity();
    ran.game.as_mut().expect("game half").version = "update:02.51".to_string();
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &ran,
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("recorded CG_TEST base")
            && failures[0].contains("ran CG_TEST update:02.51"),
        "got {failures:?}"
    );
}

/// A reinstall from another PUP keeps the console-visible version, so
/// the report carries every compared field.
#[test]
fn two_firmwares_sharing_a_version_are_still_told_apart_in_the_report() {
    let mut ran = test_identity();
    ran.firmware.as_mut().expect("firmware half").pup_sha256 = "ff".to_string();
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &ran,
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("pup sha256 00") && failures[0].contains("pup sha256 ff"),
        "got {failures:?}"
    );
}

/// The same for the game half: two trees of one version can differ in
/// the `APP_VER` their PARAM.SFO declares.
#[test]
fn two_title_trees_sharing_a_version_are_still_told_apart_in_the_report() {
    let mut ran = test_identity();
    ran.game.as_mut().expect("game half").app_ver = "01.01".to_string();
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &ran,
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("app_ver 01.00") && failures[0].contains("app_ver 01.01"),
        "got {failures:?}"
    );
}

/// An anchor can predate the install of one half of the triple.
#[test]
fn a_half_the_anchor_never_named_is_reported_as_unidentified() {
    let mut ran = test_identity();
    ran.game = None;
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &ran,
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(failures[0].contains("(unidentified)"), "got {failures:?}");
}

/// `FromStr` round-trips the Display form, so the comparison must use
/// it too: Debug renders the address in decimal and would report a
/// mismatch against an identical outcome.
#[test]
fn a_pc_reached_outcome_compares_by_its_display_form() {
    let mut baseline = anchor_fixture(73);
    baseline.outcome = BootOutcome::PcReached(0x1_0000);
    let observed = observed_stderr(73, 100);
    let same = anchor_disagreements(
        &baseline,
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "PcReached(0x10000)",
        &observed,
    );
    assert!(same.is_empty(), "identical outcome must match: {same:?}");
    let debug_form = anchor_disagreements(
        &baseline,
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "PcReached(65536)",
        &observed,
    );
    assert!(
        !debug_form.is_empty(),
        "the decimal Debug form is not equal"
    );
}

#[test]
fn a_cell_with_no_committed_anchor_is_skipped_not_failed() {
    let verdict = check_anchor(
        "CG_NO_SUCH_CONTENT_ID",
        &test_cell(),
        &measured_run("BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=0\n"),
    );
    assert_eq!(verdict, AnchorVerdict::NotRecorded("fw 4.93 x base".into()));
}

/// The anchor tree is keyed by the whole triple, so a file one path
/// segment away is another cell's anchor.
#[test]
fn a_sibling_cells_anchor_does_not_stand_in_for_an_unrecorded_one() {
    let other = CellKey {
        fw: "3.55".to_string(),
        game_ver: Some("base".to_string()),
    };
    let verdict = check_anchor(
        "NPUA80001",
        &other,
        &measured_run("BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=0\n"),
    );
    assert_eq!(verdict, AnchorVerdict::NotRecorded("fw 3.55 x base".into()));
}

/// The workspace root is compiled in, so every cell looks unrecorded
/// once the binary leaves its source tree. Saying so is the difference
/// between a reported skip and a gate that quietly stopped gating.
#[test]
fn an_unreachable_workspace_root_does_not_read_as_an_unrecorded_cell() {
    let absent = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("no_such_workspace_root");
    let verdict = check_anchor_under(
        &absent,
        "VSH",
        &test_cell(),
        &measured_run("BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=0\n"),
    );
    let AnchorVerdict::NotComparable(reasons) = verdict else {
        panic!("expected NotComparable, got {verdict:?}");
    };
    assert_eq!(reasons.len(), 1, "got {reasons:?}");
    assert!(
        reasons[0].contains("no_such_workspace_root"),
        "the reason must name the path it looked under: {}",
        reasons[0]
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

#[test]
fn an_anchor_with_no_witnesses_is_a_disagreement() {
    let mut baseline = anchor_fixture(73);
    baseline.witnesses.clear();
    let failures = anchor_disagreements(
        &baseline,
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert_eq!(failures, vec!["anchor records no witnesses".to_string()]);
}

#[test]
fn a_recorded_witness_whose_line_never_appeared_is_reported() {
    let observed = parse_witness_lines("BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=73\n")
        .expect("synthetic witness line parses");
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed,
    );
    assert!(
        failures
            .iter()
            .any(|f| f.contains("ldarx") && f.contains("BENCH_ATOMIC_WITNESS:")),
        "a missing emitter must not read as an observed zero: {failures:?}"
    );
}

#[test]
fn a_witness_the_anchor_does_not_carry_is_reported() {
    let observed = parse_witness_lines(
        "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=73\n\
         BENCH_ATOMIC_WITNESS: ldarx=100 stdcx=0 lwarx=0 stwcx=0\n\
         BENCH_DCBZ_WITNESS: count=4\n",
    )
    .expect("synthetic witness lines parse");
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed,
    );
    assert_eq!(
        failures,
        vec!["witness dcbz is emitted but not recorded in the anchor".to_string()]
    );
}

#[test]
fn a_zero_step_run_against_a_recorded_anchor_is_a_disagreement() {
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &test_identity(),
        TEST_CHECKPOINT,
        0,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert!(
        failures.iter().any(|f| f.contains("steps 0")),
        "got {failures:?}"
    );
}

fn bench_manifest(bench_max_steps: Option<u64>) -> crate::game::manifest::TitleManifest {
    use crate::game::manifest::{Distribution, GameSource};
    crate::game::manifest::TitleManifest {
        content_id: "CG_TEST".to_string(),
        short_name: "test".to_string(),
        display_name: "test".to_string(),
        eboot_candidates: vec!["EBOOT.BIN".to_string()],
        year: 2007,
        developer: "test-developer".to_string(),
        engine: "test-engine".to_string(),
        distribution: Distribution::PsnHdd,
        rap_filename: None,
        bench_max_steps,
        checkpoint: manifest::CheckpointTrigger::ProcessExit,
        source: GameSource::Hdd,
        rsx_mirror: false,
        rsx_consume: false,
        content: None,
        mounts: Vec::new(),
        matrix: Vec::new(),
    }
}

/// A run of `title` in `cell`, at exactly what the registry declares
/// for it.
fn bench_options<'a>(
    title: &'a crate::game::manifest::TitleManifest,
    cell: Option<&'a CellKey>,
    guest_args: &'a [String],
) -> BenchOptions<'a> {
    let max_steps = crate::paths::cell_max_steps(title, None);
    BenchOptions {
        title,
        elf_path: "EBOOT.BIN",
        max_steps: max_steps as usize,
        plan: AnchorPlan {
            cell,
            max_steps,
            checkpoint: title.checkpoint_trigger(),
        },
        firmware_dir: None,
        composed_mounts: &[],
        identity: &cellgov_compare::RunIdentity {
            firmware: None,
            game: None,
        },
        selection: SelectionArgs::default(),
        strict_reserved: false,
        checkpoint_override: None,
        budget_override: None,
        prescan: false,
        guest_args,
        check_anchor: true,
        run_index: 0,
    }
}

#[test]
fn a_run_at_the_recorded_cap_is_comparable() {
    let cell = test_cell();
    let title = bench_manifest(None);
    assert!(incomparable_reasons(&bench_options(&title, Some(&cell), &[])).is_empty());
    let capped = bench_manifest(Some(4_000));
    let opts = bench_options(&capped, Some(&cell), &[]);
    assert_eq!(opts.max_steps, 4_000);
    assert!(incomparable_reasons(&opts).is_empty());
}

/// A diagnostic-only flag must not disable the gate: `--prescan` only
/// prints a decode report before execution.
#[test]
fn prescan_leaves_the_run_comparable() {
    let cell = test_cell();
    let title = bench_manifest(None);
    let mut opts = bench_options(&title, Some(&cell), &[]);
    opts.prescan = true;
    assert!(incomparable_reasons(&opts).is_empty());
}

#[test]
fn a_shortened_run_is_not_compared_against_the_anchor() {
    let cell = test_cell();
    let title = bench_manifest(None);
    let mut opts = bench_options(&title, Some(&cell), &[]);
    opts.max_steps = 50_000;
    let reasons = incomparable_reasons(&opts);
    assert_eq!(reasons.len(), 1, "got {reasons:?}");
    assert!(reasons[0].contains("--max-steps 50000"), "got {reasons:?}");
}

/// The cell's cap is what its anchor was recorded at, so a run at the
/// title-level default is the retargeted one.
#[test]
fn a_run_at_the_title_cap_is_incomparable_against_a_cell_that_overrides_it() {
    let cell = test_cell();
    let title = bench_manifest(Some(4_000));
    let mut opts = bench_options(&title, Some(&cell), &[]);
    opts.plan.max_steps = 250;
    let reasons = incomparable_reasons(&opts);
    assert_eq!(reasons.len(), 1, "got {reasons:?}");
    assert!(
        reasons[0].contains("--max-steps 4000") && reasons[0].contains("250"),
        "got {reasons:?}"
    );
}

#[test]
fn every_trajectory_override_names_itself_as_incomparable() {
    let cell = test_cell();
    let title = bench_manifest(None);
    let args = vec!["EBOOT.BIN".to_string()];

    let mut checkpoint = bench_options(&title, Some(&cell), &[]);
    checkpoint.checkpoint_override = Some(manifest::CheckpointTrigger::Pc(0x1_0000));
    let mut budget = bench_options(&title, Some(&cell), &[]);
    budget.budget_override = Some(Budget::new(512));
    let mut strict = bench_options(&title, Some(&cell), &[]);
    strict.strict_reserved = true;
    let guest = bench_options(&title, Some(&cell), &args);

    for (label, opts) in [
        ("--checkpoint", checkpoint),
        ("--budget", budget),
        ("--strict-reserved", strict),
        ("--guest-arg", guest),
    ] {
        let reasons = incomparable_reasons(&opts);
        assert_eq!(reasons.len(), 1, "{label}: got {reasons:?}");
        assert!(reasons[0].contains(label), "{label}: got {reasons:?}");
    }
}

/// An anchor is filed under a cell, so a run that composed none has
/// nothing to be held against.
#[test]
fn a_run_that_composed_no_cell_is_not_compared() {
    let title = bench_manifest(None);

    let mut unmanaged = bench_options(&title, None, &[]);
    unmanaged.selection = SelectionArgs {
        firmware_dir: Some("dev_flash/sys/external"),
        ..SelectionArgs::default()
    };
    let reasons = incomparable_reasons(&unmanaged);
    assert_eq!(reasons.len(), 1, "got {reasons:?}");
    assert!(reasons[0].contains("--firmware-dir"), "got {reasons:?}");

    let reasons = incomparable_reasons(&bench_options(&title, None, &[]));
    assert_eq!(reasons.len(), 1, "got {reasons:?}");
    assert!(reasons[0].contains("composed no cell"), "got {reasons:?}");
}

/// The anchor is keyed by the firmware and the game version, so a run
/// that selects them composes the cell it is held against.
#[test]
fn selecting_a_firmware_and_a_game_version_leaves_the_run_comparable() {
    let cell = test_cell();
    let title = bench_manifest(None);
    let mut opts = bench_options(&title, Some(&cell), &[]);
    opts.selection = SelectionArgs {
        fw: Some("4.93"),
        game_ver: Some("base"),
        ..SelectionArgs::default()
    };
    assert!(incomparable_reasons(&opts).is_empty());
}

#[test]
fn the_child_receives_the_selection_flags_not_the_resolved_firmware_dir() {
    let cell = test_cell();
    let title = bench_manifest(None);
    let mut opts = bench_options(&title, Some(&cell), &[]);
    opts.firmware_dir = Some("resolved/4.91/dev_flash/sys/external");
    opts.selection = SelectionArgs {
        fw: Some("4.91"),
        game_ver: Some("02.51"),
        firmware_dir: None,
        vfs_root: Some("elsewhere/dev_hdd0"),
    };
    let mut cmd = std::process::Command::new("cellgov_cli");
    opts.encode_to_command(&mut cmd);
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let forwarded = |flag: &str, value: &str| {
        args.windows(2)
            .any(|pair| pair[0] == flag && pair[1] == value)
    };
    assert!(
        forwarded("--vfs-root", "elsewhere/dev_hdd0"),
        "got {args:?}"
    );
    assert!(forwarded("--fw", "4.91"), "got {args:?}");
    assert!(forwarded("--game-ver", "02.51"), "got {args:?}");
    assert!(
        !args.iter().any(|a| a == "--firmware-dir"),
        "the resolved module directory must not reach the child: {args:?}"
    );
}

/// Restating the cell's own checkpoint is not a retarget, so it must
/// not disable the comparison.
#[test]
fn a_checkpoint_override_equal_to_the_cells_stays_comparable() {
    let cell = test_cell();
    let title = bench_manifest(None);
    let mut opts = bench_options(&title, Some(&cell), &[]);
    opts.checkpoint_override = Some(opts.plan.checkpoint);
    assert!(incomparable_reasons(&opts).is_empty());
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

/// Every `"BENCH_<NAME>:` string literal in `source`: the prefixes of
/// the stderr lines the boot path emits. A literal inside an
/// `#[error(...)]` attribute is an error's Display text, not a line.
fn emitted_bench_prefixes(source: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (start, _) in source.match_indices("\"BENCH_") {
        if source[..start].trim_end().ends_with("#[error(") {
            continue;
        }
        let body = &source[start + 1..];
        let name_len = body
            .bytes()
            .take_while(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b'_')
            .count();
        if body[name_len..].starts_with(':') {
            out.insert(body[..=name_len].to_string());
        }
    }
    out
}

/// The boot stage modules the `BENCH_` scan reads.
///
/// `include_str!` takes a literal path, so the list is hand-written;
/// [`the_bench_line_scan_reads_every_boot_stage_module`] holds it
/// against the directory.
const BOOT_SOURCES: [(&str, &str); 11] = [
    ("entry.rs", include_str!("../boot/entry.rs")),
    ("finish.rs", include_str!("../boot/finish.rs")),
    ("firmware.rs", include_str!("../boot/firmware.rs")),
    ("host.rs", include_str!("../boot/host.rs")),
    ("image.rs", include_str!("../boot/image.rs")),
    ("loaders.rs", include_str!("../boot/loaders.rs")),
    ("module_start.rs", include_str!("../boot/module_start.rs")),
    ("params.rs", include_str!("../boot/params.rs")),
    ("prepare.rs", include_str!("../boot/prepare.rs")),
    ("providers.rs", include_str!("../boot/providers.rs")),
    ("types.rs", include_str!("../boot/types.rs")),
];

#[test]
fn the_bench_line_scan_reads_every_boot_stage_module() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/game/boot");
    let mut on_disk: BTreeSet<String> = BTreeSet::new();
    for entry in std::fs::read_dir(&dir).expect("boot stage directory") {
        let path = entry.expect("boot stage directory entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("boot stage module file name")
            .to_string();
        // mod.rs carries module declarations and re-exports only.
        if name != "mod.rs" {
            on_disk.insert(name);
        }
    }
    let scanned: BTreeSet<String> = BOOT_SOURCES.iter().map(|(n, _)| (*n).to_string()).collect();
    assert_eq!(
        on_disk, scanned,
        "boot stage modules the BENCH_ line scan does not read"
    );
}

#[test]
fn every_emitted_bench_line_is_tracked_or_reasoned_diagnostic() {
    let mut emitted = BTreeSet::new();
    for source in [
        include_str!("../bench.rs"),
        include_str!("../child_init.rs"),
        include_str!("../prx/module_start.rs"),
    ]
    .into_iter()
    .chain(BOOT_SOURCES.iter().map(|(_, source)| *source))
    {
        emitted.extend(emitted_bench_prefixes(source));
    }
    assert!(
        emitted.len() > 20,
        "the scan found only {emitted:?}; the literal shape it keys on has moved"
    );

    let tracked: BTreeSet<String> = cellgov_compare::witness_parse::tracked_line_prefixes()
        .into_iter()
        .map(str::to_string)
        .collect();
    let diagnostic: BTreeSet<String> = cellgov_compare::witness_parse::diagnostic_lines()
        .iter()
        .map(|(p, _)| (*p).to_string())
        .collect();

    let unclassified: Vec<&String> = emitted
        .iter()
        .filter(|p| !tracked.contains(*p) && !diagnostic.contains(*p))
        .collect();
    assert!(
        unclassified.is_empty(),
        "emitted BENCH_ lines with no witness and no stated diagnostic-only reason: {unclassified:?}"
    );

    let stale: Vec<&String> = tracked
        .iter()
        .chain(diagnostic.iter())
        .filter(|p| !emitted.contains(*p))
        .collect();
    assert!(
        stale.is_empty(),
        "line-table rows no emitter produces: {stale:?}"
    );
}
