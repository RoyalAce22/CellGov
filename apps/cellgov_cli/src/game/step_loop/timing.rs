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
#[path = "tests/timing_tests.rs"]
mod tests;
