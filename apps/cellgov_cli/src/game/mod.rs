//! `run-game` subcommand: boot a PS3 ELF and drive the PPU step loop.

mod bench;
mod boot;
mod content;
mod diag;
pub mod manifest;
mod mounts;
mod observation;
mod prescan_format;
mod prx;
mod run;
mod stack_walk;
mod step_loop;

pub use bench::{bench_boot_one_run, bench_boot_pair, BenchGate, BenchOptions};
pub use run::{run_game, RunGameOptions, RunSummary};
