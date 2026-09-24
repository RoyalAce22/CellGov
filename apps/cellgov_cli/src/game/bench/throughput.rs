//! The throughput half of a run set: what a spread of wall times
//! supports, and how the set prints the claim.

use std::time::Duration;

use cellgov_compare::bench::BenchBootResult;

/// Cross-run wall spread above which a run set makes no throughput
/// claim, as a percentage of the fastest run.
///
/// A spread above the ceiling is a nonzero exit only under
/// `--strict-perf`. On a busy host the spread measures the host.
pub const BENCH_SPREAD_CEILING_PCT: f64 = 5.0;

/// How the throughput half of a run set behaves.
#[derive(Debug, Clone, Copy)]
pub struct ThroughputPolicy {
    /// Subprocess measurements to take.
    pub runs: usize,
    /// Turn a throughput verdict the set could not reach into a
    /// nonzero exit. Set it only on an idle host.
    pub strict: bool,
}

/// What the throughput half of a run set concluded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ThroughputVerdict {
    /// Spread within [`BENCH_SPREAD_CEILING_PCT`]. `min` estimates the
    /// uncontended cost: contention only ever adds time, so the
    /// fastest run is the least contaminated one.
    Measured { min: Duration, spread_pct: f64 },
    /// Spread above the ceiling, so the set makes no throughput claim.
    Inconclusive { min: Duration, spread_pct: f64 },
    /// A run reported a zero wall, so there is no spread to compare.
    Unmeasurable,
}

impl ThroughputVerdict {
    pub(super) fn is_measured(self) -> bool {
        matches!(self, Self::Measured { .. })
    }
}

pub(super) fn print_throughput(verdict: ThroughputVerdict, policy: ThroughputPolicy) {
    println!("{}", throughput_line(verdict, policy));
}

/// A set of one run spreads against nothing, and a range over one
/// measurement is 0%. That would read as an agreement, so the
/// single-run line names the estimate and says the spread went
/// unchecked.
fn throughput_line(verdict: ThroughputVerdict, policy: ThroughputPolicy) -> String {
    let outcome = if policy.strict && !verdict.is_measured() {
        "FAIL (--strict-perf)"
    } else if verdict.is_measured() {
        "OK"
    } else {
        "SKIPPED"
    };
    match verdict {
        ThroughputVerdict::Measured { min, .. } if policy.runs < 2 => format!(
            "  throughput: min_ms={:.3} over 1 run, spread NOT CHECKED -- one \
             measurement spreads against nothing => {outcome}",
            min.as_secs_f64() * 1e3,
        ),
        ThroughputVerdict::Measured { min, spread_pct } => format!(
            "  throughput: min_ms={:.3} over {} run(s), spread {spread_pct:.2}% \
             (ceiling {BENCH_SPREAD_CEILING_PCT}%) => {outcome}",
            min.as_secs_f64() * 1e3,
            policy.runs,
        ),
        ThroughputVerdict::Inconclusive { min, spread_pct } => format!(
            "  throughput: INCONCLUSIVE -- min_ms={:.3} over {} run(s), spread \
             {spread_pct:.2}% above the {BENCH_SPREAD_CEILING_PCT}% ceiling; the host was \
             busy, so no throughput claim is made => {outcome}",
            min.as_secs_f64() * 1e3,
            policy.runs,
        ),
        ThroughputVerdict::Unmeasurable => format!(
            "  throughput: INCONCLUSIVE -- a run reported a zero wall, so there is no \
             spread to compare => {outcome}"
        ),
    }
}

pub(super) fn throughput_verdict(runs: &[BenchBootResult]) -> ThroughputVerdict {
    if runs.is_empty() {
        return ThroughputVerdict::Unmeasurable;
    }
    let mut min = Duration::MAX;
    let mut max = Duration::ZERO;
    for run in runs {
        if run.wall.is_zero() {
            return ThroughputVerdict::Unmeasurable;
        }
        min = min.min(run.wall);
        max = max.max(run.wall);
    }
    let spread_pct = 100.0 * (max.as_secs_f64() - min.as_secs_f64()) / min.as_secs_f64();
    if spread_pct > BENCH_SPREAD_CEILING_PCT {
        ThroughputVerdict::Inconclusive { min, spread_pct }
    } else {
        ThroughputVerdict::Measured { min, spread_pct }
    }
}

#[cfg(test)]
#[path = "tests/throughput_tests.rs"]
mod tests;
