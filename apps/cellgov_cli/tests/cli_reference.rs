//! The help an operator reads, and the two generators that publish it.
//! Needs no corpus.

use std::path::{Path, PathBuf};
use std::process::Command;

fn cellgov(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args(args)
        .output()
        .expect("spawn cellgov")
}

fn help(path: &[&str]) -> String {
    let mut args = path.to_vec();
    args.push("--help");
    let out = cellgov(&args);
    assert!(out.status.success(), "{path:?} --help exits 0");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("apps/cellgov_cli has a workspace root two levels up")
        .to_path_buf()
}

/// The verbs a help text lists under `Commands:`, without clap's own
/// `help`.
fn verbs(text: &str) -> Vec<String> {
    let Some(section) = text.split("Commands:\n").nth(1) else {
        return Vec::new();
    };
    section
        .lines()
        .take_while(|l| !l.trim().is_empty())
        .filter_map(|l| l.split_whitespace().next())
        .filter(|name| *name != "help")
        .map(str::to_string)
        .collect()
}

/// Every command path that takes arguments and does not dispatch to a
/// verb, found by a walk over the binary's own help.
fn leaf_paths() -> Vec<Vec<String>> {
    let mut out = Vec::new();
    walk(&[], &mut out);
    out
}

fn walk(path: &[String], out: &mut Vec<Vec<String>>) {
    let borrowed: Vec<&str> = path.iter().map(String::as_str).collect();
    let text = help(&borrowed);
    let verbs = verbs(&text);
    if verbs.is_empty() {
        if !path.is_empty() {
            out.push(path.to_vec());
        }
        return;
    }
    for verb in verbs {
        let mut child = path.to_vec();
        child.push(verb);
        walk(&child, out);
    }
}

/// Every command path below `cellgov` that only dispatches to verbs,
/// found by a walk over the binary's own help.
fn noun_paths() -> Vec<Vec<String>> {
    let mut out = Vec::new();
    walk_nouns(&[], &mut out);
    out
}

fn walk_nouns(path: &[String], out: &mut Vec<Vec<String>>) {
    let borrowed: Vec<&str> = path.iter().map(String::as_str).collect();
    let text = help(&borrowed);
    let verbs = verbs(&text);
    if verbs.is_empty() {
        return;
    }
    // `explore` carries a positional of its own alongside its verbs, so
    // its help leads with examples the way a leaf's does.
    if !path.is_empty() && !text.contains("\nArguments:\n") {
        out.push(path.to_vec());
    }
    for verb in verbs {
        let mut child = path.to_vec();
        child.push(verb);
        walk_nouns(&child, out);
    }
}

/// Every command path the committed reference gives a section, as the
/// path an operator types.
fn documented_paths() -> Vec<Vec<String>> {
    let doc = std::fs::read_to_string(repo_root().join("docs/cli.md")).expect("read docs/cli.md");
    doc.lines()
        .filter(|l| l.starts_with('#'))
        .filter_map(|l| l.split('`').nth(1).map(str::to_string))
        .filter_map(|p| {
            p.strip_prefix("cellgov").map(|rest| {
                rest.split_whitespace()
                    .map(str::to_string)
                    .collect::<Vec<String>>()
            })
        })
        .collect()
}

/// The documented paths no deeper section extends.
fn documented_leaves() -> Vec<Vec<String>> {
    let all = documented_paths();
    all.iter()
        .filter(|p| !p.is_empty())
        .filter(|p| {
            !all.iter()
                .any(|q| q.len() > p.len() && q.starts_with(p.as_slice()))
        })
        .cloned()
        .collect()
}

#[test]
fn the_help_walk_finds_exactly_the_leaves_the_reference_documents() {
    let mut walked = leaf_paths();
    let mut documented = documented_leaves();
    assert!(
        documented.len() > 30,
        "the reference documents {} leaves",
        documented.len()
    );
    walked.sort();
    documented.sort();
    assert_eq!(
        walked, documented,
        "the help walk and the reference disagree about the command tree"
    );
}

#[test]
fn a_noun_that_only_dispatches_verbs_prints_no_examples() {
    let nouns = noun_paths();
    assert!(nouns.len() > 5, "the walk found {} nouns", nouns.len());
    for noun in nouns {
        let borrowed: Vec<&str> = noun.iter().map(String::as_str).collect();
        let text = help(&borrowed);
        assert!(
            !text.contains("\nExamples:\n"),
            "{noun:?} dispatches verbs and should lead with them:\n{text}"
        );
        assert!(
            text.contains("\nCommands:\n"),
            "{noun:?} lists no verbs:\n{text}"
        );
    }
}

#[test]
fn a_command_with_both_a_positional_and_verbs_still_leads_with_examples() {
    let text = help(&["explore"]);
    let examples = text
        .find("\nExamples:\n")
        .unwrap_or_else(|| panic!("explore help has no examples block:\n{text}"));
    for heading in ["\nCommands:\n", "\nArguments:\n", "\nOptions:\n"] {
        let at = text
            .find(heading)
            .unwrap_or_else(|| panic!("explore help has no {heading:?}:\n{text}"));
        assert!(examples < at, "examples belong before {heading:?}:\n{text}");
    }
}

#[test]
fn every_leaf_help_leads_with_examples_before_the_flag_table() {
    let leaves = leaf_paths();
    assert!(leaves.len() > 30, "the walk found {} leaves", leaves.len());
    for leaf in leaves {
        let borrowed: Vec<&str> = leaf.iter().map(String::as_str).collect();
        let text = help(&borrowed);
        let examples = text
            .find("\nExamples:\n")
            .unwrap_or_else(|| panic!("{leaf:?} help has no examples block:\n{text}"));
        let usage = text.find("\nUsage: ").expect("every help states its usage");
        assert!(
            usage < examples,
            "{leaf:?}: examples belong after the usage line:\n{text}"
        );
        for heading in ["\nOptions:\n", "\nArguments:\n"] {
            if let Some(at) = text.find(heading) {
                assert!(
                    examples < at,
                    "{leaf:?}: examples belong before {heading:?}:\n{text}"
                );
            }
        }
    }
}

#[test]
fn every_example_the_help_prints_invokes_the_command_it_appears_under() {
    let leaves = leaf_paths();
    assert!(leaves.len() > 30, "the walk found {} leaves", leaves.len());
    for leaf in leaves {
        let borrowed: Vec<&str> = leaf.iter().map(String::as_str).collect();
        let text = help(&borrowed);
        let block = text
            .split("\nExamples:\n")
            .nth(1)
            .expect("the leaf leads with examples");
        let expected = format!("cellgov {}", leaf.join(" "));
        let lines: Vec<&str> = block
            .lines()
            .take_while(|l| !l.trim().is_empty())
            .map(str::trim)
            .collect();
        assert!(
            (1..=3).contains(&lines.len()),
            "{leaf:?} prints {} examples",
            lines.len()
        );
        for line in lines {
            assert!(
                line.starts_with(&expected),
                "{line:?} is not a {expected:?}"
            );
        }
    }
}

#[test]
fn cli_gen_writes_the_committed_reference() {
    let out_dir = std::env::temp_dir().join(format!("cellgov_cli_gen_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out_dir);
    std::fs::create_dir_all(&out_dir).expect("create temp dir");
    let generated = out_dir.join("cli.md");

    let out = cellgov(&[
        "dev",
        "cli-gen",
        "--output",
        &generated.display().to_string(),
    ]);
    assert!(
        out.status.success(),
        "cli-gen failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let committed = std::fs::read_to_string(repo_root().join("docs/cli.md")).expect("read docs");
    let written = std::fs::read_to_string(&generated).expect("read generated");
    assert_eq!(
        normalize(&committed),
        normalize(&written),
        "docs/cli.md is stale; regenerate with `cellgov dev cli-gen`"
    );
    let _ = std::fs::remove_dir_all(&out_dir);
}

/// Stands in for a region that switches on the `decrypt` feature.
///
/// A binary built with `decrypt` renders a document the committed one
/// cannot equal. Both sides blank those regions, and every other byte
/// still has to match. Mirrors the same helper in the in-crate
/// `drift_tests` gate.
const FEATURE_DEPENDENT: &str = "<feature-dependent>";
const SCE_NOTE_HEADING: &str = "SCE-wrapped input:";
const VFS_ROOT_ROW: &str = "| `--vfs-root` |";

fn normalize(text: &str) -> String {
    let mut out = String::new();
    let mut in_note = false;
    for line in text.replace("\r\n", "\n").lines() {
        if line == SCE_NOTE_HEADING {
            in_note = true;
            out.push_str(FEATURE_DEPENDENT);
            out.push('\n');
            continue;
        }
        if in_note {
            if line.starts_with("  ") {
                continue;
            }
            in_note = false;
        }
        if line.starts_with(VFS_ROOT_ROW) {
            out.push_str(FEATURE_DEPENDENT);
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[test]
fn the_feature_dependent_regions_are_found_in_the_committed_document() {
    let committed = std::fs::read_to_string(repo_root().join("docs/cli.md")).expect("read docs");
    assert!(committed.contains(SCE_NOTE_HEADING));
    assert!(committed.contains(VFS_ROOT_ROW));
    assert!(normalize(&committed).matches(FEATURE_DEPENDENT).count() >= 5);
}

#[test]
fn completions_reach_stdout_for_every_shell() {
    let names: Vec<String> = documented_paths()
        .into_iter()
        .filter_map(|p| p.last().cloned())
        .collect();
    assert!(
        names.len() > 40,
        "the reference names {} commands",
        names.len()
    );
    for (shell, marker) in [
        ("bash", "_cellgov()"),
        ("zsh", "#compdef cellgov"),
        ("pwsh", "Register-ArgumentCompleter"),
    ] {
        let out = cellgov(&["dev", "completions", shell]);
        assert!(out.status.success(), "{shell}: {:?}", out.status);
        let script = String::from_utf8_lossy(&out.stdout);
        assert!(script.contains(marker), "{shell} script has no {marker:?}");
        for name in &names {
            assert!(
                script.contains(name.as_str()),
                "{shell} script does not name {name}"
            );
        }
        assert!(
            script.contains("--no-anchor-check"),
            "{shell} script names no leaf flag"
        );
        assert!(
            out.stderr.is_empty(),
            "{shell} wrote to stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn an_unknown_shell_is_a_usage_error() {
    let out = cellgov(&["dev", "completions", "fish"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("bash"),
        "the refusal lists what it takes:\n{stderr}"
    );
}
