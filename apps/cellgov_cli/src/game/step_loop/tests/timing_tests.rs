//! Untracked-time computation from loop and per-bucket durations.

use super::*;
use std::time::Duration;

#[test]
fn untracked_is_loop_minus_tracked_sum_in_happy_path() {
    let t_loop = Duration::from_millis(100);
    let step = Duration::from_millis(40);
    let commit = Duration::from_millis(20);
    let coverage = Duration::from_millis(10);
    assert_eq!(
        compute_untracked(t_loop, step, commit, coverage),
        Ok(Duration::from_millis(30))
    );
}

#[test]
fn untracked_zero_when_buckets_fill_the_loop() {
    let t_loop = Duration::from_millis(100);
    let step = Duration::from_millis(60);
    let commit = Duration::from_millis(30);
    let coverage = Duration::from_millis(10);
    assert_eq!(
        compute_untracked(t_loop, step, commit, coverage),
        Ok(Duration::ZERO)
    );
}

#[test]
fn untracked_errors_when_tracked_exceeds_loop() {
    let t_loop = Duration::from_millis(100);
    let step = Duration::from_millis(60);
    let commit = Duration::from_millis(30);
    let coverage = Duration::from_millis(25);
    assert_eq!(
        compute_untracked(t_loop, step, commit, coverage),
        Err(Duration::from_millis(15))
    );
}

#[test]
fn untracked_handles_zero_loop_cleanly() {
    assert_eq!(
        compute_untracked(
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO
        ),
        Ok(Duration::ZERO)
    );
}

#[test]
fn untracked_saturates_on_arithmetic_overflow() {
    let result = compute_untracked(
        Duration::from_millis(100),
        Duration::MAX,
        Duration::from_millis(1),
        Duration::from_millis(1),
    );
    assert!(result.is_err());
}
