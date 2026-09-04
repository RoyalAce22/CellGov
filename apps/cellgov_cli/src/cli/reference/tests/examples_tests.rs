//! Example blocks against the tree they document.

use super::examples::{lines_for, EXAMPLES, MAX_EXAMPLES};
use super::*;

/// Space-separated path below `cellgov` for every command that takes
/// arguments and does not dispatch to a verb.
fn leaf_paths() -> Vec<String> {
    let mut out = Vec::new();
    collect_leaves(&command_tree(), "", &mut out);
    out
}

fn collect_leaves(cmd: &clap::Command, path: &str, out: &mut Vec<String>) {
    let verbs: Vec<&clap::Command> = cmd
        .get_subcommands()
        .filter(|s| s.get_name() != "help")
        .collect();
    // `explore` both takes a positional and carries `micro`, so a
    // command with verbs is still a leaf when it has positionals of
    // its own.
    let has_own_args = cmd.get_arguments().any(|a| a.is_positional());
    if verbs.is_empty() || has_own_args {
        out.push(path.to_string());
    }
    for sub in verbs {
        let child = match path {
            "" => sub.get_name().to_string(),
            _ => format!("{path} {}", sub.get_name()),
        };
        collect_leaves(sub, &child, out);
    }
}

/// The tree as `--help` renders it.
///
/// A global flag reaches a subcommand only once the tree is built,
/// and the layout follows.
fn built() -> clap::Command {
    let mut tree = command_tree();
    tree.build();
    tree
}

/// `line` as argv.
///
/// A separate test holds every example quote-free, so a plain
/// whitespace split is enough.
fn argv(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_string).collect()
}

/// `path`'s command in the built tree, ready to render its own help.
fn resolve(tree: &clap::Command, path: &str) -> Option<clap::Command> {
    let mut cmd = tree;
    for name in path.split_whitespace() {
        cmd = cmd.find_subcommand(name)?;
    }
    Some(cmd.clone())
}

/// Space-separated path below `cellgov` for every command that only
/// dispatches to verbs, so its help is the verb list.
fn noun_paths() -> Vec<String> {
    let mut out = Vec::new();
    collect_nouns(&command_tree(), "", &mut out);
    out
}

fn collect_nouns(cmd: &clap::Command, path: &str, out: &mut Vec<String>) {
    let verbs: Vec<&clap::Command> = cmd
        .get_subcommands()
        .filter(|s| s.get_name() != "help")
        .collect();
    let has_own_args = cmd.get_arguments().any(|a| a.is_positional());
    if !verbs.is_empty() && !has_own_args && !path.is_empty() {
        out.push(path.to_string());
    }
    for sub in verbs {
        let child = match path {
            "" => sub.get_name().to_string(),
            _ => format!("{path} {}", sub.get_name()),
        };
        collect_nouns(sub, &child, out);
    }
}

#[test]
fn every_leaf_declares_examples() {
    let missing: Vec<String> = leaf_paths()
        .into_iter()
        .filter(|p| lines_for(p).is_none())
        .collect();
    assert!(
        missing.is_empty(),
        "leaf commands with no example block: {missing:?}"
    );
}

#[test]
fn the_root_declares_examples() {
    assert!(lines_for("").is_some());
}

#[test]
fn the_table_declares_a_block_for_more_than_thirty_commands() {
    assert!(
        EXAMPLES.len() > 30,
        "the table declares {} blocks",
        EXAMPLES.len()
    );
}

#[test]
fn every_example_block_holds_one_to_three_invocations() {
    for e in EXAMPLES {
        assert!(
            (1..=MAX_EXAMPLES).contains(&e.lines.len()),
            "{:?} declares {} examples",
            e.path,
            e.lines.len()
        );
    }
}

#[test]
fn no_two_example_entries_name_the_same_command() {
    let mut seen: Vec<&str> = EXAMPLES.iter().map(|e| e.path).collect();
    let declared = seen.len();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(
        seen.len(),
        declared,
        "a second entry for a path is unreachable: the lookup answers with the first"
    );
}

#[test]
fn no_example_line_is_blank() {
    for e in EXAMPLES {
        for line in e.lines {
            assert!(
                !line.trim().is_empty(),
                "{:?} holds a blank invocation, which renders as a bare prompt",
                e.path
            );
        }
    }
}

#[test]
fn every_example_names_the_command_it_documents() {
    for e in EXAMPLES {
        let expected = format!("cellgov {}", e.path);
        let expected = expected.trim_end();
        for line in e.lines {
            assert!(
                line.starts_with(expected),
                "{line:?} does not invoke {expected:?}"
            );
        }
    }
}

#[test]
fn no_example_needs_shell_quoting() {
    for e in EXAMPLES {
        for line in e.lines {
            assert!(
                !line.contains(['"', '\'', '|', '>', '<', '$']),
                "{line:?} would not survive the whitespace split into argv"
            );
        }
    }
}

#[test]
fn every_example_parses_against_the_command_tree() {
    let mut checked = 0usize;
    for e in EXAMPLES {
        for line in e.lines {
            let parsed = crate::cli::parse::try_parse(&argv(line));
            let cli = match parsed {
                Ok(cli) => cli,
                Err(e) => panic!("{line:?} does not parse: {e}"),
            };
            // Parsing is half the answer. `parse_or_exit` then holds
            // the globals against the command and exits 2 on a
            // mismatch, so an example can parse and still fail.
            assert_eq!(
                crate::cli::parse::global_refusal(&cli),
                None,
                "{line:?} parses but the binary refuses it"
            );
            checked += 1;
        }
    }
    assert!(checked > 30, "only {checked} example lines were parsed");
}

#[test]
fn no_example_path_is_absent_from_the_tree() {
    let leaves = leaf_paths();
    for e in EXAMPLES {
        assert!(
            e.path.is_empty() || leaves.iter().any(|p| p == e.path),
            "{:?} names no leaf of the command tree",
            e.path
        );
    }
}

#[test]
fn examples_reach_the_help_of_the_command_that_declares_them() {
    let tree = built();
    let mut checked = 0usize;
    for e in EXAMPLES {
        let mut cmd = resolve(&tree, e.path)
            .unwrap_or_else(|| panic!("{:?} names no command in the tree", e.path));
        let help = cmd.render_help().to_string();
        let examples = help
            .find("Examples:")
            .unwrap_or_else(|| panic!("{:?} help leads with no examples:\n{help}", e.path));
        for heading in ["\nCommands:\n", "\nArguments:\n", "\nOptions:\n"] {
            if let Some(at) = help.find(heading) {
                assert!(
                    examples < at,
                    "{:?}: examples belong before {heading:?}:\n{help}",
                    e.path
                );
            }
        }
        for line in e.lines {
            assert!(help.contains(line), "{line:?} missing from:\n{help}");
        }
        checked += 1;
    }
    assert!(checked > 30, "only {checked} example blocks were checked");
}

#[test]
fn a_noun_leads_with_its_verbs_rather_than_examples() {
    let tree = built();
    let nouns = noun_paths();
    assert!(nouns.len() > 5, "the walk found {} nouns", nouns.len());
    for noun in nouns {
        assert!(
            lines_for(&noun).is_none(),
            "{noun:?} dispatches verbs and declares an example block"
        );
        let mut cmd = resolve(&tree, &noun).expect("the walk names commands in the tree");
        let help = cmd.render_help().to_string();
        assert!(!help.contains("Examples:"), "{noun:?}:\n{help}");
        assert!(help.contains("Commands:"), "{noun:?}:\n{help}");
    }
}

#[test]
fn the_root_help_fits_one_screen() {
    let mut tree = built();
    // `render_help` is what the binary prints for both `-h` and
    // `--help`. No argument declares a long help, so clap has no
    // second layout.
    let help = tree.render_help().to_string();
    let lines = help.lines().count();
    assert!(lines <= 50, "cellgov --help is {lines} lines:\n{help}");
}

#[test]
fn the_root_help_lists_dev_last() {
    let mut tree = built();
    let help = tree.render_help().to_string();
    let dev = help.find("\n  dev ").expect("dev is listed");
    let nouns: Vec<String> = command_tree()
        .get_subcommands()
        .map(|s| s.get_name().to_string())
        .filter(|n| n != "help" && n != "dev")
        .collect();
    assert!(
        nouns.len() > 5,
        "the tree declares {} top-level nouns",
        nouns.len()
    );
    for noun in nouns {
        let at = help
            .find(&format!("\n  {noun} "))
            .unwrap_or_else(|| panic!("{noun} is listed:\n{help}"));
        assert!(at < dev, "{noun} must be listed before dev:\n{help}");
    }
}
