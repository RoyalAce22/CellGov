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
        "  3    runs that had to reproduce each other disagreed",
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

/// `--save-state-trace` and `--run-index` reach `boot bench` through
/// the argument struct it shares with `boot bench-once`. The refusal
/// lands before any store read.
#[test]
fn a_child_only_bench_flag_on_the_run_set_is_a_usage_error() {
    for extra in [
        vec!["--save-state-trace", "trace.state"],
        vec!["--run-index", "2"],
    ] {
        let mut argv = vec!["boot", "bench", "--title", "synthetic"];
        argv.extend_from_slice(&extra);
        let out = Command::new(env!("CARGO_BIN_EXE_cellgov"))
            .args(&argv)
            .output()
            .expect("spawn cellgov");
        assert_eq!(out.status.code(), Some(2), "{argv:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("bench-once"),
            "{argv:?}: the refusal names the command that takes the flag:\n{stderr}"
        );
    }
}

/// The refusal lands before any store read.
#[test]
fn a_strict_throughput_gate_over_one_run_is_a_usage_error() {
    let out = Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args([
            "boot",
            "bench",
            "--title",
            "synthetic",
            "--runs",
            "1",
            "--strict-perf",
        ])
        .output()
        .expect("spawn cellgov");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--strict-perf") && stderr.contains("--runs 1"),
        "the refusal names both flags it is about:\n{stderr}"
    );
}

#[test]
fn a_strict_throughput_gate_over_several_runs_is_not_refused() {
    let out = Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args([
            "boot",
            "bench",
            "--title",
            "synthetic",
            "--runs",
            "2",
            "--strict-perf",
        ])
        .output()
        .expect("spawn cellgov");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("--strict-perf enforces"),
        "a two-run set measures a spread, so nothing is refused:\n{stderr}"
    );
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
