//! Dispatch coverage: every command the tree declares reaches a
//! handler.

use clap::CommandFactory as _;

use super::*;

/// Every leaf `dispatch` and `dispatch_dev` route, as the path an
/// operator types.
fn declared_paths() -> Vec<String> {
    let mut out = Vec::new();
    collect(&Cli::command(), &mut Vec::new(), &mut out);
    out
}

fn collect(command: &clap::Command, prefix: &mut Vec<String>, out: &mut Vec<String>) {
    let mut leaf = true;
    for sub in command.get_subcommands() {
        // `help` is clap's own, not one this binary dispatches.
        if sub.get_name() == "help" {
            continue;
        }
        leaf = false;
        prefix.push(sub.get_name().to_string());
        collect(sub, prefix, out);
        prefix.pop();
    }
    if leaf && !prefix.is_empty() {
        out.push(prefix.join(" "));
    }
}

/// The routes `dispatch` matches on, spelled the way an operator types
/// them. A command declared in the tree and missing here would parse
/// and then reach no handler.
const DISPATCHED: &[&str] = &[
    "firmware install",
    "title install",
    "title install-update",
    "title uninstall",
    "keys show",
    "keys import",
    "keys remove",
    "self decrypt",
    "boot run",
    "boot bench",
    "boot bench-once",
    "diff compare",
    "diff observations",
    "diff diverge",
    "diff zoom",
    "explore",
    "explore micro",
    "scenario list",
    "scenario run",
    "scenario dump",
    "dev disasm",
    "dev prx-imports",
    "dev funcs",
    "dev rpcs3-attribute",
    "dev fixture-gen",
    "dev titles-gen",
    "dev gen-manifest",
    "dev record-anchors",
];

#[test]
fn every_declared_command_is_dispatched() {
    let declared = declared_paths();
    for path in &declared {
        assert!(
            DISPATCHED.contains(&path.as_str()),
            "{path} is in the tree but not in the dispatch table",
        );
    }
    for path in DISPATCHED {
        // `explore` is the one command with both a positional form and
        // a subcommand, so the walk reports only the subcommand.
        if *path == "explore" {
            continue;
        }
        assert!(
            declared.contains(&(*path).to_string()),
            "{path} is dispatched but no longer in the tree",
        );
    }
}

#[test]
fn a_scenario_name_no_longer_shadows_a_top_level_command() {
    let command = Cli::command();
    let top: Vec<&str> = command
        .get_subcommands()
        .map(clap::Command::get_name)
        .collect();
    for name in SCENARIOS {
        assert!(
            !top.contains(name),
            "scenario {name:?} collides with a top-level command",
        );
    }
}
