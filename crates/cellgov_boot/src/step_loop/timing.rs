//! Per-bucket wall-clock accounting for the diagnostic step loop.
//!
//! Display only: nothing here feeds a scheduling decision or a state
//! hash.

/// Where the diagnostic loop's wall time went.
#[derive(Debug, Clone, Copy, Default)]
pub struct StepTiming {
    /// Time inside `Runtime::step`.
    pub step_time: std::time::Duration,
    /// Time inside `Runtime::commit_step`.
    pub commit_time: std::time::Duration,
    /// Time inside the per-instruction coverage tally.
    pub coverage_time: std::time::Duration,
}

/// Untracked time = `t_loop - (step + commit + coverage)`.
///
/// # Errors
///
/// Returns `Err(excess)` when tracked buckets exceed `t_loop` -- bucket
/// overlap, double-counting, or non-monotonic clock.
pub fn compute_untracked(
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

/// `part` as a percentage of `total`; a zero total reads as 0.
pub fn pct(part: std::time::Duration, total: std::time::Duration) -> f64 {
    if total.is_zero() {
        0.0
    } else {
        100.0 * part.as_secs_f64() / total.as_secs_f64()
    }
}

#[cfg(test)]
#[path = "tests/timing_tests.rs"]
mod tests;
