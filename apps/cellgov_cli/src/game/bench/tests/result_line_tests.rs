use std::time::Duration;

use cellgov_compare::BootOutcome;
use cellgov_time::Budget;

use super::super::throughput::throughput_verdict;
use super::*;

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
