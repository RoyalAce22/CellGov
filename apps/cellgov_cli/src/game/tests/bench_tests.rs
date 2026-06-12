//! Bench-result line parsing and wall-clock disagreement math.

use super::*;

#[test]
fn wall_disagreement_percent_is_zero_for_identical_durations() {
    use std::time::Duration;
    assert_eq!(
        wall_disagreement_percent(Duration::from_millis(1000), Duration::from_millis(1000)),
        Some(0.0)
    );
}

#[test]
fn wall_disagreement_percent_is_relative_to_faster_run() {
    use std::time::Duration;
    let pct = wall_disagreement_percent(Duration::from_millis(100), Duration::from_millis(105))
        .expect("finite");
    assert!((pct - 5.0).abs() < 0.0001, "expected 5.0, got {pct}");
}

#[test]
fn wall_disagreement_percent_is_symmetric() {
    use std::time::Duration;
    let a = wall_disagreement_percent(Duration::from_millis(200), Duration::from_millis(250));
    let b = wall_disagreement_percent(Duration::from_millis(250), Duration::from_millis(200));
    assert_eq!(a, b);
}

#[test]
fn wall_disagreement_percent_returns_none_on_zero_duration() {
    use std::time::Duration;
    assert_eq!(
        wall_disagreement_percent(Duration::ZERO, Duration::from_millis(100)),
        None
    );
    assert_eq!(
        wall_disagreement_percent(Duration::from_millis(100), Duration::ZERO),
        None
    );
    assert_eq!(
        wall_disagreement_percent(Duration::ZERO, Duration::ZERO),
        None
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
        let line = format!("BENCH_RESULT steps=1 wall_ms=1 steps_per_sec=1000 outcome={v}\n");
        let r = parse_bench_result(&line)
            .unwrap_or_else(|e| panic!("round-trip parse failed for {v:?}: {e}"));
        assert_eq!(r.outcome, v, "round-trip mismatch for {v:?}");
    }
}

#[test]
fn parse_bench_result_extracts_fields() {
    let stdout = "some preamble\nBENCH_RESULT steps=1402388 wall_ms=323 steps_per_sec=4342377 outcome=ProcessExit\ntrailing noise\n";
    let r = parse_bench_result(stdout).expect("parses");
    assert_eq!(r.steps, 1402388);
    assert_eq!(r.wall.as_millis(), 323);
    assert_eq!(r.outcome, BootOutcome::ProcessExit);
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
    let stdout = "BENCH_RESULT steps=1 wall_ms=1 steps_per_sec=1 outcome=ProcessExit\n\
                  BENCH_RESULT steps=2 wall_ms=2 steps_per_sec=1 outcome=ProcessExit\n";
    assert_eq!(
        parse_bench_result(stdout).unwrap_err(),
        ParseBenchError::DuplicateResultLine
    );
}

#[test]
fn parse_bench_result_errors_on_unknown_outcome() {
    let stdout = "BENCH_RESULT steps=1 wall_ms=1 steps_per_sec=1 outcome=WhoKnows\n";
    match parse_bench_result(stdout).unwrap_err() {
        ParseBenchError::UnparseableOutcome { token, source: _ } => {
            assert_eq!(token, "WhoKnows");
        }
        other => panic!("expected UnparseableOutcome, got {other:?}"),
    }
}

#[test]
fn parse_bench_result_errors_on_malformed_steps() {
    let stdout = "BENCH_RESULT steps=abc wall_ms=1 steps_per_sec=1 outcome=ProcessExit\n";
    match parse_bench_result(stdout).unwrap_err() {
        ParseBenchError::MalformedSteps(s) => assert_eq!(s, "abc"),
        other => panic!("expected MalformedSteps, got {other:?}"),
    }
}

#[test]
fn parse_bench_result_errors_on_missing_steps() {
    let stdout = "BENCH_RESULT wall_ms=1 steps_per_sec=1 outcome=ProcessExit\n";
    assert_eq!(
        parse_bench_result(stdout).unwrap_err(),
        ParseBenchError::MissingSteps
    );
}

#[test]
fn parse_bench_result_errors_on_malformed_wall_ms() {
    let stdout = "BENCH_RESULT steps=1 wall_ms=xyz steps_per_sec=1 outcome=ProcessExit\n";
    match parse_bench_result(stdout).unwrap_err() {
        ParseBenchError::MalformedWallMs(s) => assert_eq!(s, "xyz"),
        other => panic!("expected MalformedWallMs, got {other:?}"),
    }
}

#[test]
fn parse_bench_result_errors_on_missing_wall_ms() {
    let stdout = "BENCH_RESULT steps=1 steps_per_sec=1 outcome=ProcessExit\n";
    assert_eq!(
        parse_bench_result(stdout).unwrap_err(),
        ParseBenchError::MissingWallMs
    );
}

#[test]
fn parse_bench_result_errors_on_missing_outcome() {
    let stdout = "BENCH_RESULT steps=1 wall_ms=1 steps_per_sec=1\n";
    assert_eq!(
        parse_bench_result(stdout).unwrap_err(),
        ParseBenchError::MissingOutcome
    );
}

#[test]
fn classify_pair_pass() {
    use std::time::Duration;
    let r1 = BenchBootResult {
        steps: 10,
        wall: Duration::from_millis(100),
        outcome: BootOutcome::ProcessExit,
    };
    let r2 = BenchBootResult {
        steps: 10,
        wall: Duration::from_millis(102),
        outcome: BootOutcome::ProcessExit,
    };
    let drift = wall_disagreement_percent(r1.wall, r2.wall);
    assert_eq!(classify_pair(&r1, &r2, drift), BenchGate::Pass);
}

#[test]
fn classify_pair_determinism_break_on_step_mismatch() {
    use std::time::Duration;
    let r1 = BenchBootResult {
        steps: 10,
        wall: Duration::from_millis(100),
        outcome: BootOutcome::ProcessExit,
    };
    let r2 = BenchBootResult {
        steps: 11,
        wall: Duration::from_millis(100),
        outcome: BootOutcome::ProcessExit,
    };
    let drift = wall_disagreement_percent(r1.wall, r2.wall);
    assert_eq!(classify_pair(&r1, &r2, drift), BenchGate::DeterminismBreak);
}

#[test]
fn classify_pair_determinism_break_on_outcome_mismatch() {
    use std::time::Duration;
    let r1 = BenchBootResult {
        steps: 10,
        wall: Duration::from_millis(100),
        outcome: BootOutcome::ProcessExit,
    };
    let r2 = BenchBootResult {
        steps: 10,
        wall: Duration::from_millis(100),
        outcome: BootOutcome::MaxSteps,
    };
    let drift = wall_disagreement_percent(r1.wall, r2.wall);
    assert_eq!(classify_pair(&r1, &r2, drift), BenchGate::DeterminismBreak);
}

#[test]
fn classify_pair_wall_drift_exceeded() {
    use std::time::Duration;
    let r1 = BenchBootResult {
        steps: 10,
        wall: Duration::from_millis(100),
        outcome: BootOutcome::ProcessExit,
    };
    let r2 = BenchBootResult {
        steps: 10,
        wall: Duration::from_millis(200),
        outcome: BootOutcome::ProcessExit,
    };
    let drift = wall_disagreement_percent(r1.wall, r2.wall);
    assert_eq!(classify_pair(&r1, &r2, drift), BenchGate::WallDriftExceeded);
}

#[test]
fn classify_pair_wall_unmeasurable() {
    use std::time::Duration;
    let r1 = BenchBootResult {
        steps: 10,
        wall: Duration::ZERO,
        outcome: BootOutcome::ProcessExit,
    };
    let r2 = BenchBootResult {
        steps: 10,
        wall: Duration::from_millis(100),
        outcome: BootOutcome::ProcessExit,
    };
    assert_eq!(classify_pair(&r1, &r2, None), BenchGate::WallUnmeasurable);
}
