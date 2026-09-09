//! The result of one bench run, and the verdict a run set reaches over
//! all of its runs.

use std::time::Duration;

use cellgov_compare::BootOutcome;
use cellgov_time::Budget;

use super::anchor::AnchorVerdict;
use super::throughput::ThroughputVerdict;

/// One completed bench run.
#[derive(Debug, Clone, Copy)]
pub struct BenchBootResult {
    pub run_index: usize,
    pub steps: usize,
    pub wall: Duration,
    /// Instructions each step was granted; `steps * budget` is the
    /// count the run retired.
    pub budget: Budget,
    pub outcome: BootOutcome,
}

impl BenchBootResult {
    pub fn steps_per_sec(&self) -> f64 {
        let secs = self.wall.as_secs_f64();
        if secs == 0.0 {
            0.0
        } else {
            self.steps as f64 / secs
        }
    }
}

/// Gate verdict for [`bench_boot_runs`](super::bench_boot_runs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchGate {
    /// Every run reproduced the same steps, outcome and witness map,
    /// and the anchor comparison found nothing.
    Pass,
    /// Runs disagreed on retired step count, boot outcome, or a
    /// witness.
    DeterminismBreak,
    /// The run disagreed with the cell's committed anchor.
    AnchorDrift,
    /// The set reached no throughput claim under `--strict-perf`.
    SpreadExceeded,
}

/// Result of one [`bench_boot_runs`](super::bench_boot_runs) invocation.
#[derive(Debug, Clone)]
pub struct BenchRunsOutcome {
    /// Every measurement taken, in the order they ran.
    pub runs: Vec<BenchBootResult>,
    pub throughput: ThroughputVerdict,
    pub gate: BenchGate,
    /// How run 1 compared against the cell's anchor.
    ///
    /// A [`BenchGate::AnchorDrift`] gate always carries
    /// [`AnchorVerdict::Drift`]. A determinism break outranks the
    /// anchor, so a drift can also sit under
    /// [`BenchGate::DeterminismBreak`].
    pub anchor: AnchorVerdict,
    /// Every way the runs failed to reproduce each other, empty unless
    /// `gate` is [`BenchGate::DeterminismBreak`].
    pub determinism_failures: Vec<String>,
}
