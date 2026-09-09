//! When the render loop predicts an end, and against what.

use super::*;

const STEP_FLOOR: f64 = 1.0;
const SETTLED: Duration = Duration::from_secs(3);

#[test]
fn the_eta_predicts_the_declared_finish_line_from_the_rate() {
    // 30,000 steps to go at 1,000 steps/s.
    assert_eq!(
        eta_secs(SETTLED, 1_000.0, STEP_FLOOR, 10_000, 40_000),
        Some(30)
    );
    // A fractional second rounds up, never to a prediction of zero.
    assert_eq!(
        eta_secs(SETTLED, 3_000.0, STEP_FLOOR, 39_999, 40_000),
        Some(1)
    );
}

#[test]
fn a_phase_with_no_denominator_predicts_nothing() {
    assert_eq!(eta_secs(SETTLED, 1_000_000.0, STEP_FLOOR, 43_040, 0), None);
}

#[test]
fn a_run_at_or_past_its_finish_line_predicts_nothing() {
    assert_eq!(eta_secs(SETTLED, 1_000.0, STEP_FLOOR, 40_000, 40_000), None);
    assert_eq!(eta_secs(SETTLED, 1_000.0, STEP_FLOOR, 44_100, 40_000), None);
}

#[test]
fn a_rate_at_the_floor_or_a_first_second_predicts_nothing() {
    assert_eq!(eta_secs(SETTLED, STEP_FLOOR, STEP_FLOOR, 0, 40_000), None);
    assert_eq!(
        eta_secs(Duration::from_millis(999), 1_000.0, STEP_FLOOR, 0, 40_000),
        None
    );
    assert_eq!(
        eta_secs(Duration::from_secs(1), 1_000.0, STEP_FLOOR, 0, 40_000),
        Some(40)
    );
}
