//! Which `explore` form reads `--vfs-root`.

use clap::Parser as _;

use super::globals::{global_refusal, reads_vfs_root};
use super::{Cli, Command};

/// The parsed command of `argv`, with the program name prepended.
fn command_of(argv: &[&str]) -> Command {
    let mut full = vec!["cellgov"];
    full.extend_from_slice(argv);
    Cli::try_parse_from(full)
        .expect("clap accepts this invocation")
        .command
}

#[test]
fn explore_title_reads_the_vfs_root_the_boot_family_does() {
    let command = command_of(&["explore", "title", "--title", "synthetic"]);
    assert!(reads_vfs_root(&command));
}

#[test]
fn a_scenario_or_microtest_exploration_reads_no_vfs_root() {
    for argv in [
        vec!["explore", "fairness"],
        vec!["explore", "micro", "barrier_wakeup"],
    ] {
        assert!(!reads_vfs_root(&command_of(&argv)), "{argv:?}");
    }
}

#[test]
fn the_vfs_root_refusal_names_explore_title_among_the_readers() {
    let cli = Cli::try_parse_from(["cellgov", "--vfs-root", "v", "explore", "fairness"])
        .expect("clap accepts this invocation");
    let said = global_refusal(&cli).expect("a scenario exploration reads no store");
    assert!(said.contains("explore title"), "{said}");
}
