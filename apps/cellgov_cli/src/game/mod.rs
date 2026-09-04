//! Boot a PS3 title ELF and drive the PPU step loop for the
//! `boot run` and `boot bench` subcommands.

mod bench;
mod boot;
mod child_init;
mod content;
mod diag;
mod guest_args;
pub mod manifest;
mod mounts;
mod observation;
mod prescan_format;
mod prx;
mod run;
mod stack_walk;
mod step_loop;

pub use bench::{
    bench_boot_one_run, bench_boot_runs, AnchorPlan, BenchGate, BenchOptions, SelectionArgs,
    ThroughputPolicy, ThroughputVerdict, BENCH_DEFAULT_RUNS, BENCH_SPREAD_CEILING_PCT,
};
pub use run::{run_game, RunGameOptions, RunSummary};
