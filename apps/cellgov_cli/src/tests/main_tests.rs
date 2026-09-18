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
    "status",
    "firmware install",
    "firmware list",
    "firmware show",
    "firmware verify",
    "firmware verify-corpus",
    "firmware kernels",
    "firmware uninstall",
    "title install",
    "title install-update",
    "title list",
    "title show",
    "title verify",
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
    "explore title",
    "scenario list",
    "scenario run",
    "scenario dump",
    "dev disasm",
    "dev prx-imports",
    "dev funcs",
    #[cfg(feature = "decrypt")]
    "dev lv2-extract",
    #[cfg(feature = "decrypt")]
    "dev caller-census",
    "dev rpcs3-attribute",
    "dev fixture-gen",
    "dev titles-gen",
    "dev cli-gen",
    "dev completions",
    "dev gen-manifest",
    "dev record-anchors",
];

#[test]
#[cfg(not(feature = "decrypt"))]
fn default_help_exposes_no_decrypt_dev_entry_points() {
    let command = Cli::command();
    let dev = command
        .find_subcommand("dev")
        .expect("the tree declares dev");
    assert!(dev.find_subcommand("lv2-extract").is_none());
    assert!(dev.find_subcommand("caller-census").is_none());
}

#[test]
#[cfg(feature = "decrypt")]
fn decrypt_help_exposes_caller_census_scope_and_output() {
    let mut command = Cli::command();
    command.build();
    let dev = command
        .find_subcommand("dev")
        .expect("the tree declares dev");
    let mut census = dev
        .find_subcommand("caller-census")
        .expect("the decrypt tree declares dev caller-census")
        .clone();
    let help = census.render_long_help().to_string();
    assert!(help.contains("--all"), "{help}");
    assert!(help.contains("--fw <VERSION>"), "{help}");
    assert!(help.contains("--output-dir <DIR>"), "{help}");
}

#[test]
#[cfg(feature = "decrypt")]
fn decrypt_help_exposes_lv2_extract_selection_and_output() {
    let mut command = Cli::command();
    command.build();
    let dev = command
        .find_subcommand("dev")
        .expect("the tree declares dev");
    let mut extract = dev
        .find_subcommand("lv2-extract")
        .expect("the decrypt tree declares dev lv2-extract")
        .clone();
    let help = extract.render_long_help().to_string();
    assert!(help.contains("--fw <VERSION>"), "{help}");
    assert!(help.contains("--output-dir <DIR>"), "{help}");
    assert!(help.contains("only installed"), "{help}");
}

#[test]
#[cfg(feature = "decrypt")]
fn lv2_extract_accepts_the_vfs_root_and_json_globals() {
    let argv: Vec<String> = [
        "cellgov",
        "--vfs-root",
        "store/dev_hdd0",
        "--format",
        "json",
        "dev",
        "lv2-extract",
        "--fw",
        "4.93",
        "--output-dir",
        "output",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    let cli = parse::try_parse(&argv).expect("the extraction invocation parses");
    assert_eq!(parse::global_refusal(&cli), None);
}

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
fn format_refusal_names_verify_corpus_as_a_supported_reader() {
    let argv: Vec<String> = ["cellgov", "--format", "json", "scenario", "list"]
        .into_iter()
        .map(str::to_string)
        .collect();
    let cli = parse::try_parse(&argv).expect("the invocation parses before global validation");
    let refusal = parse::global_refusal(&cli).expect("scenario list does not render a report");
    assert!(refusal.contains("verify-corpus"), "{refusal}");
}

#[test]
fn verify_corpus_help_limits_installed_checks_to_archive_backed_entries() {
    let mut command = Cli::command();
    command.build();
    let firmware = command
        .find_subcommand("firmware")
        .expect("the tree declares firmware");
    let mut verify_corpus = firmware
        .find_subcommand("verify-corpus")
        .expect("the tree declares firmware verify-corpus")
        .clone();
    let help = verify_corpus.render_long_help().to_string();
    assert!(
        help.contains("whose PUP hash names an archive row matched"),
        "{help}"
    );
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

/// `explore` carries both a `SCENARIO` positional and verbs, and clap
/// reads a bare word as a verb before it reaches the positional. A verb
/// spelled like a scenario would make that scenario unreachable.
#[test]
fn a_scenario_name_no_longer_shadows_an_explore_verb() {
    let command = Cli::command();
    let explore = command
        .find_subcommand("explore")
        .expect("the tree declares explore");
    let verbs: Vec<&str> = explore
        .get_subcommands()
        .map(clap::Command::get_name)
        .collect();
    assert!(
        verbs.len() > 1,
        "explore declares {} verb(s), so the walk found nothing to check",
        verbs.len()
    );
    for name in SCENARIOS {
        assert!(
            !verbs.contains(name),
            "scenario {name:?} collides with an explore verb, which clap resolves first",
        );
    }
}
