//! Drive the boot library for the `boot run` and `boot bench`
//! subcommands: these modules take the parsed options and render the
//! report the library produces.

mod bench;
mod finish_line;
mod run;
mod sink;

pub use bench::{
    bench_boot_one_run, bench_boot_runs, AnchorPlan, AnchorVerdict, BenchGate, BenchOptions,
    BenchRunsOutcome, SelectionArgs, ThroughputPolicy, ThroughputVerdict, BENCH_DEFAULT_RUNS,
    BENCH_SPREAD_CEILING_PCT,
};
pub(crate) use finish_line::{anchor_finish_line, within_runtime_cap};
pub use run::{run_game, RunArtifacts, RunExecution, RunReporting, RunSummary};
pub(crate) use sink::console_sink;
