//! Which `boot run` flags move the run off its anchor's trajectory.

use clap::Parser as _;

use super::*;
use crate::cli::parse::{BootCommand, Cli, Command};

fn run_args(extra: &[&str]) -> BootRunArgs {
    let mut argv = vec!["cellgov", "boot", "run", "--title", "synthetic"];
    argv.extend_from_slice(extra);
    let cli = Cli::try_parse_from(&argv).unwrap_or_else(|e| panic!("{argv:?}: {e}"));
    let Command::Boot(BootCommand::Run(args)) = cli.command else {
        panic!("{argv:?} did not select `boot run`");
    };
    *args
}

#[test]
fn a_plain_boot_run_retraces_the_anchor() {
    assert!(!run_retargets_anchor(&run_args(&[]), false));
}

#[test]
fn diagnostics_and_the_cap_retarget_nothing() {
    let args = run_args(&[
        "--max-steps",
        "50000",
        "--trace",
        "--profile",
        "--profile-pairs",
        "--prescan",
        "--save-observation",
        "obs.json",
        "--observation-manifest",
        "regions.toml",
        "--save-boot-summary",
        "summary.json",
        "--save-state-trace",
        "trace.bin",
        "--dump-mem-boot",
        "0x10000",
        "--dump-mem-fault",
        "0x10000:0x100",
    ]);
    assert!(!run_retargets_anchor(&args, false));
}

#[test]
fn every_trajectory_override_retargets_the_finish_line() {
    for (label, extra) in [
        ("an executable override", &["EBOOT.BIN"][..]),
        ("--budget", &["--budget", "512"]),
        ("--strict-reserved", &["--strict-reserved"]),
        ("--guest-arg", &["--guest-arg", "argv1"]),
        ("--patch-byte", &["--patch-byte", "0x10000=0x60"]),
        ("--dump-at-pc", &["--dump-at-pc", "0x10000"]),
    ] {
        assert!(run_retargets_anchor(&run_args(extra), false), "{label}");
    }
}

#[test]
fn a_cell_recorded_at_another_checkpoint_retargets_the_finish_line() {
    assert!(run_retargets_anchor(&run_args(&[]), true));
}

#[test]
fn a_cell_at_the_checkpoint_boot_run_stops_at_is_where_it_ends() {
    use cellgov_boot::manifest::CheckpointTrigger;
    for cp in [
        CheckpointTrigger::ProcessExit,
        CheckpointTrigger::FirstRsxWrite,
    ] {
        assert!(run_ends_at_cell_checkpoint(cp, cp), "{}", cp.as_cli_str());
    }
    assert!(!run_ends_at_cell_checkpoint(
        CheckpointTrigger::ProcessExit,
        CheckpointTrigger::FirstRsxWrite
    ));
}

#[test]
fn a_pc_checkpoint_is_not_where_boot_run_ends() {
    use cellgov_boot::manifest::CheckpointTrigger;
    let pc = CheckpointTrigger::Pc(0x1_0000);
    assert!(!run_ends_at_cell_checkpoint(pc, pc));
}
