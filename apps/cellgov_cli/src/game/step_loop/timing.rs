#[derive(Default)]
pub(in crate::game) struct StepTiming {
    pub(in crate::game) step_time: std::time::Duration,
    pub(in crate::game) commit_time: std::time::Duration,
    pub(in crate::game) coverage_time: std::time::Duration,
}

/// Untracked time = `t_loop - (step + commit + coverage)`.
///
/// # Errors
///
/// Returns `Err(excess)` when tracked buckets exceed `t_loop` -- bucket
/// overlap, double-counting, or non-monotonic clock.
pub(in crate::game) fn compute_untracked(
    t_loop: std::time::Duration,
    step: std::time::Duration,
    commit: std::time::Duration,
    coverage: std::time::Duration,
) -> Result<std::time::Duration, std::time::Duration> {
    let tracked = step
        .checked_add(commit)
        .and_then(|s| s.checked_add(coverage))
        .unwrap_or(std::time::Duration::MAX);
    if tracked <= t_loop {
        Ok(t_loop - tracked)
    } else {
        Err(tracked - t_loop)
    }
}

pub(in crate::game) fn pct(part: std::time::Duration, total: std::time::Duration) -> f64 {
    if total.is_zero() {
        0.0
    } else {
        100.0 * part.as_secs_f64() / total.as_secs_f64()
    }
}

#[cfg(test)]
mod tests {
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
}
