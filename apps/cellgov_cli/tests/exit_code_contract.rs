//! The exit-code contract as an operator reads it: what `--help`
//! prints, and that a command with an outcome of its own names it.
//! Self-contained.

use std::process::Command;

use cellgov_testkit::scratch::scratch_labeled;

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
        (vec!["diff", "compare"], "3", "disagreed"),
        (vec!["keys", "show"], "40", "key missing"),
        (vec!["diff", "zoom"], "30", "step"),
    ] {
        let text = help(&args);
        assert!(
            text.contains("Exit codes"),
            "{args:?} has no exit-code section:
{text}"
        );
        // A whole-text search would match a stray digit elsewhere in
        // the help, or the word in some option's prose. The check reads
        // the row that starts with the code.
        let row = text
            .lines()
            .find(|line| line.trim_start().starts_with(&format!("{code} ")))
            .unwrap_or_else(|| {
                panic!(
                    "{args:?} lists no {code} row:
{text}"
                )
            });
        assert!(
            row.contains(what),
            "{args:?}: the {code} row does not say {what:?}:
{row}"
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
    // 4 (a verification diverged) and 3 (runs that had to reproduce
    // each other disagreed) are shared statuses. A command that gives
    // one does not claim it as its own.
    for args in [
        vec!["firmware", "verify"],
        vec!["title", "verify"],
        vec!["diff", "compare"],
    ] {
        let text = help(&args);
        assert!(
            !text.contains("particular to this command"),
            "{args:?} labels a shared status as its own:
{text}"
        );
    }
}

/// Two manifests that name no scenario this runner has: one without a
/// `[cellgov]` section, and one whose `[cellgov]` names an unregistered
/// scenario. Neither runs anything.
const UNSUPPORTED_MANIFESTS: [(&str, &str); 2] = [
    (
        "no_cellgov_section",
        r#"
[test]
name = "no_cellgov_section"

[observe]

[expect]
outcome = "completed"
"#,
    ),
    (
        "unknown_scenario",
        r#"
[test]
name = "unknown_scenario"

[cellgov]
scenario = "no_such_scenario"

[observe]

[expect]
outcome = "completed"
"#,
    ),
];

/// The 0 row of `diff compare --help`: `Classification::exits_failure`
/// gives UNSUPPORTED no failure.
#[test]
fn an_unsupported_manifest_is_reported_and_exits_success() {
    let dir = scratch_labeled("compare_unsupported");
    for (label, manifest) in UNSUPPORTED_MANIFESTS {
        let path = dir.join(format!("{label}.toml"));
        std::fs::write(&path, manifest).expect("write manifest");
        let out = Command::new(env!("CARGO_BIN_EXE_cellgov"))
            .args(["diff", "compare"])
            .arg(&path)
            .output()
            .expect("spawn cellgov");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("classification: UNSUPPORTED"),
            "{label}: the report does not classify UNSUPPORTED:\n{stdout}\nstderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            out.status.code(),
            Some(0),
            "{label}: an unsupported manifest ran nothing, so nothing failed:\n{stdout}"
        );
    }
}

/// The 1 row of `diff compare --help`; the refusal lands before any
/// run starts.
#[test]
fn an_unknown_bare_scenario_is_a_failed_operation() {
    let out = Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args(["diff", "compare", "no_such_scenario"])
        .output()
        .expect("spawn cellgov");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unknown scenario: no_such_scenario") && stderr.contains("available:"),
        "the refusal names the scenario and lists the available ones:\n{stderr}"
    );
}

/// `--observations-dir` reads a manifest's region descriptors, which a
/// bare scenario has none of. The refusal lands before the scenario
/// lookup, so an existing and an unknown scenario exit the same.
#[test]
fn an_observations_dir_on_a_bare_scenario_is_a_usage_error() {
    for scenario in ["dma", "no_such_scenario"] {
        let out = Command::new(env!("CARGO_BIN_EXE_cellgov"))
            .args(["diff", "compare", scenario, "--observations-dir", "."])
            .output()
            .expect("spawn cellgov");
        assert_eq!(out.status.code(), Some(2), "{scenario}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("--observations-dir") && stderr.contains("manifest.toml"),
            "{scenario}: the refusal names the flag and what it applies to:\n{stderr}"
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
