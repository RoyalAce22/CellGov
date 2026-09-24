//! The verdict a run set reaches over all of its runs.

use cellgov_compare::bench::{AnchorVerdict, BenchBootResult, BenchGate};

use super::throughput::ThroughputVerdict;

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
