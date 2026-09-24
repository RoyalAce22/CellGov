use std::time::Duration;

use cellgov_time::Budget;

use super::*;

fn parse(stdout: &str) -> Result<BenchBootResult, ParseBenchError> {
    parse_bench_result(stdout).map(|p| p.result)
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
        let r = parse(&line).unwrap_or_else(|e| panic!("round-trip parse failed for {v:?}: {e}"));
        assert_eq!(r.outcome, v, "round-trip mismatch for {v:?}");
    }
}

#[test]
fn parse_bench_result_extracts_fields() {
    let stdout = "some preamble\nBENCH_RESULT run_index=2 steps=1402388 wall_ns=323000000 steps_per_sec=4341759 budget=256 outcome=ProcessExit\ntrailing noise\n";
    let parsed = parse_bench_result(stdout).expect("parses");
    assert!(parsed.warnings.is_empty(), "got {:?}", parsed.warnings);
    let r = parsed.result;
    assert_eq!(r.run_index, 2);
    assert_eq!(r.steps, 1402388);
    assert_eq!(r.wall.as_millis(), 323);
    assert_eq!(r.budget, Budget::new(256));
    assert_eq!(r.outcome, BootOutcome::ProcessExit);
}

#[test]
fn the_result_line_round_trips_every_field() {
    for index in [0usize, 1, 7] {
        for ns in [
            1u64,
            400,
            750,
            999_999,
            1_000_001,
            1_234_567_891,
            3_037_000_123,
        ] {
            let r = BenchBootResult {
                run_index: index,
                steps: 12345,
                wall: Duration::from_nanos(ns),
                budget: Budget::new(256),
                outcome: BootOutcome::MaxSteps,
            };
            let parsed =
                parse_bench_result(&format!("{}\n", format_bench_result(&r))).expect("parses");
            assert_eq!(parsed.result, r, "run_index={index} wall_ns={ns}");
            assert!(parsed.warnings.is_empty(), "got {:?}", parsed.warnings);
        }
    }
}

#[test]
fn the_printed_steps_per_sec_is_the_rounded_quotient() {
    let r = BenchBootResult {
        run_index: 0,
        steps: 390_435,
        wall: Duration::from_nanos(3_038_513_400),
        budget: Budget::new(256),
        outcome: BootOutcome::MaxSteps,
    };
    // 390435 / 3.0385134 s = 128495.40...
    assert_eq!(r.steps_per_sec(), 128_495);
    assert!(format_bench_result(&r).contains(" steps_per_sec=128495 "));
    // Half a unit rounds up: 3 steps in 2 s is 1.5 per second.
    let half = BenchBootResult {
        steps: 3,
        wall: Duration::from_secs(2),
        ..r
    };
    assert_eq!(half.steps_per_sec(), 2);
}

#[test]
fn a_zero_wall_reports_a_zero_rate() {
    let r = BenchBootResult {
        run_index: 0,
        steps: 5,
        wall: Duration::ZERO,
        budget: Budget::new(256),
        outcome: BootOutcome::MaxSteps,
    };
    assert_eq!(r.steps_per_sec(), 0);
}

#[test]
fn parse_bench_result_errors_on_missing_run_index() {
    let stdout = "BENCH_RESULT steps=1 wall_ns=1 steps_per_sec=1 budget=256 outcome=ProcessExit\n";
    assert_eq!(parse(stdout).unwrap_err(), ParseBenchError::MissingRunIndex);
}

#[test]
fn parse_bench_result_errors_on_malformed_run_index() {
    let stdout =
        "BENCH_RESULT run_index=last steps=1 wall_ns=1 steps_per_sec=1 budget=256 outcome=ProcessExit\n";
    assert_eq!(
        parse(stdout).unwrap_err(),
        ParseBenchError::MalformedRunIndex("last".to_string())
    );
}

#[test]
fn a_wall_beyond_u64_nanoseconds_is_malformed_not_clamped() {
    let stdout =
        "BENCH_RESULT run_index=0 steps=1 wall_ns=99999999999999999999999 steps_per_sec=1 budget=256 outcome=ProcessExit\n";
    assert_eq!(
        parse(stdout).unwrap_err(),
        ParseBenchError::MalformedWallNs("99999999999999999999999".to_string())
    );
}

#[test]
fn parse_bench_result_errors_on_missing_line() {
    let stdout = "just some noise\nbut no result line\n";
    assert_eq!(parse(stdout).unwrap_err(), ParseBenchError::NoResultLine);
}

/// The prefix is a whole token: a line that only starts with the same
/// letters is not a result line.
#[test]
fn a_longer_token_sharing_the_prefix_is_not_a_result_line() {
    let stdout = "BENCH_RESULTS run_index=0 steps=1 wall_ns=1 budget=256 outcome=ProcessExit\n";
    assert_eq!(parse(stdout).unwrap_err(), ParseBenchError::NoResultLine);
}

#[test]
fn parse_bench_result_errors_on_duplicate_line() {
    let stdout = "BENCH_RESULT run_index=0 steps=1 wall_ns=1 steps_per_sec=1 budget=256 outcome=ProcessExit\n\
                  BENCH_RESULT run_index=1 steps=2 wall_ns=2 steps_per_sec=1 budget=256 outcome=ProcessExit\n";
    assert_eq!(
        parse(stdout).unwrap_err(),
        ParseBenchError::DuplicateResultLine
    );
}

#[test]
fn parse_bench_result_errors_on_unknown_outcome() {
    let stdout =
        "BENCH_RESULT run_index=0 steps=1 wall_ns=1 steps_per_sec=1 budget=256 outcome=WhoKnows\n";
    match parse(stdout).unwrap_err() {
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
    assert_eq!(
        parse(stdout).unwrap_err(),
        ParseBenchError::MalformedSteps("abc".to_string())
    );
}

#[test]
fn parse_bench_result_errors_on_missing_steps() {
    let stdout =
        "BENCH_RESULT run_index=0 wall_ns=1 steps_per_sec=1 budget=256 outcome=ProcessExit\n";
    assert_eq!(parse(stdout).unwrap_err(), ParseBenchError::MissingSteps);
}

#[test]
fn parse_bench_result_errors_on_malformed_wall_ns() {
    let stdout =
        "BENCH_RESULT run_index=0 steps=1 wall_ns=xyz steps_per_sec=1 budget=256 outcome=ProcessExit\n";
    assert_eq!(
        parse(stdout).unwrap_err(),
        ParseBenchError::MalformedWallNs("xyz".to_string())
    );
}

#[test]
fn parse_bench_result_errors_on_missing_wall_ns() {
    let stdout =
        "BENCH_RESULT run_index=0 steps=1 steps_per_sec=1 budget=256 outcome=ProcessExit\n";
    assert_eq!(parse(stdout).unwrap_err(), ParseBenchError::MissingWallNs);
}

#[test]
fn parse_bench_result_errors_on_missing_outcome() {
    let stdout = "BENCH_RESULT run_index=0 steps=1 wall_ns=1 steps_per_sec=1 budget=256\n";
    assert_eq!(parse(stdout).unwrap_err(), ParseBenchError::MissingOutcome);
}

#[test]
fn parse_bench_result_errors_on_missing_budget() {
    let stdout = "BENCH_RESULT run_index=0 steps=1 wall_ns=1 steps_per_sec=1 outcome=ProcessExit\n";
    assert_eq!(parse(stdout).unwrap_err(), ParseBenchError::MissingBudget);
}

#[test]
fn parse_bench_result_errors_on_malformed_budget() {
    let stdout = "BENCH_RESULT run_index=0 steps=1 wall_ns=1 steps_per_sec=1 budget=lots outcome=ProcessExit\n";
    assert_eq!(
        parse(stdout).unwrap_err(),
        ParseBenchError::MalformedBudget("lots".to_string())
    );
}

/// A skipped token is data the caller renders, never a print from
/// inside the reader.
#[test]
fn a_skipped_token_comes_back_as_a_warning() {
    let stdout = "BENCH_RESULT run_index=0 steps=1 wall_ns=1 steps_per_sec=fast budget=256 outcome=ProcessExit shiny=yes\n";
    let parsed =
        parse_bench_result(stdout).expect("a redundant or unknown token does not reject the line");
    assert_eq!(
        parsed.warnings,
        vec![
            BenchLineWarning::MalformedStepsPerSec("fast".to_string()),
            BenchLineWarning::UnknownToken("shiny=yes".to_string()),
        ]
    );
    assert!(parsed.warnings[1].to_string().contains("shiny=yes"));
}
