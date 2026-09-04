use std::time::Duration;

use super::super::anchor::AnchorVerdict;
use super::super::runs::classify_runs;
use super::super::test_fixtures::{reporting, set_of};
use super::super::types::BenchGate;
use super::*;

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

/// The run set asserts a nonzero count before it measures, so only
/// this case reaches the guard on the `Duration::MAX` seed.
#[test]
fn a_set_with_no_runs_leaves_the_throughput_unmeasurable() {
    assert_eq!(throughput_verdict(&[]), ThroughputVerdict::Unmeasurable);
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
