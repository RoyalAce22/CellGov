//! `boot bench` / `boot bench-once` machinery.
//!
//! A run set takes N subprocess measurements. It gates on what the
//! runs must reproduce exactly: steps, outcome, budget, and witness
//! map. It reports throughput separately, because elapsed time
//! measures the host as much as the emulator.

mod anchor;
mod divergence;
mod options;
mod run_one;
mod runs;
mod spawn;
mod throughput;
mod types;
mod witnesses;

#[cfg(test)]
#[path = "tests/test_fixtures.rs"]
mod test_fixtures;

pub use cellgov_compare::bench::{AnchorVerdict, BenchGate};
pub use options::{AnchorPlan, BenchOptions, SelectionArgs, BENCH_DEFAULT_RUNS};
pub use run_one::bench_boot_one_run;
pub use runs::bench_boot_runs;
pub use throughput::{ThroughputPolicy, ThroughputVerdict, BENCH_SPREAD_CEILING_PCT};

#[allow(unused_imports, reason = "reached through an entry point's signature")]
pub use cellgov_compare::bench::{BenchBootResult, ParseBenchError};
#[allow(unused_imports, reason = "reached through an entry point's signature")]
pub use spawn::SpawnError;
#[allow(unused_imports, reason = "reached through an entry point's signature")]
pub use types::BenchRunsOutcome;
