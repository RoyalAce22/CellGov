//! The exit-code contract as an operator reads it: what `--help`
//! prints, and that a command with an outcome of its own names it.
//! Needs no corpus.

use std::process::Command;

fn help(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args(args)
        .arg("--help")
        .output()
        .expect("spawn cellgov");
    assert!(out.status.success(), "--help exits 0");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn the_top_level_help_states_the_shared_contract() {
    let text = help(&[]);
    for line in [
        "  0    success",
        "  1    the operation ran and failed",
        "  2    usage error",
        "  3    the two runs of a pair disagreed",
        "  4    a subprocess failed, or a verification diverged",
        "  5    a boot moved off its committed anchor",
    ] {
        assert!(
            text.contains(line),
            "the help has no line {line:?}:\n{text}"
        );
    }
}

#[test]
fn a_command_whose_help_lists_an_exit_code_lists_it_in_the_shared_form() {
    // A bare digit turns up in prose and in paths, so every check below
    // is on the code as the exit-code list writes it: indented, then its
    // description.
    for (args, code, what) in [
        (vec!["firmware", "verify"], "4", "diverged"),
        (vec!["title", "verify"], "4", "diverged"),
        (vec!["keys", "show"], "40", "key missing"),
        (vec!["diff", "zoom"], "30", "step"),
    ] {
        let text = help(&args);
        assert!(
            text.contains("Exit codes"),
            "{args:?} has no exit-code section:
{text}"
        );
        assert!(
            text.contains(&format!("  {code} ")),
            "{args:?} lists no {code} row:
{text}"
        );
        assert!(
            text.contains(what),
            "{args:?} does not say {what:?}:
{text}"
        );
    }
}

/// Only a code the shared contract does not define is particular to one
/// command, so only those commands may say so.
#[test]
fn only_a_command_specific_code_is_labelled_particular_to_that_command() {
    for args in [vec!["keys", "show"], vec!["diff", "zoom"]] {
        let text = help(&args);
        assert!(
            text.contains("Exit codes particular to this command"),
            "{args:?} gives a code above the shared range and does not say so:
{text}"
        );
    }
    // 4 is the shared "a verification diverged", so a verify command
    // names which shared status it gives rather than claiming its own.
    for args in [vec!["firmware", "verify"], vec!["title", "verify"]] {
        let text = help(&args);
        assert!(
            !text.contains("particular to this command"),
            "{args:?} labels a shared status as its own:
{text}"
        );
    }
}

#[test]
fn an_unknown_verb_is_a_usage_error_and_runs_nothing() {
    let out = Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args(["title", "lst"])
        .output()
        .expect("spawn cellgov");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("list"),
        "the refusal suggests the nearest verb:\n{stderr}"
    );
}
