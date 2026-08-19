//! Boot a PS3 title ELF and drive the PPU step loop for the
//! `run-game` and bench-boot subcommands.

mod bench;
mod boot;
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
    bench_boot_one_run, bench_boot_pair, BenchGate, BenchOptions, BENCH_AGREEMENT_GATE_PCT,
};
pub use run::{run_game, RunGameOptions, RunSummary};
