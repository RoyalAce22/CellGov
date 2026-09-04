//! The renderer never changes what stdout says.
//!
//! The progress bar is only adoptable across the boot family if
//! turning it on cannot move a byte of the result stream. Each case
//! runs the same boot twice -- once with the bar (stderr is a pipe
//! here, so it renders in `Plain`) and once under `--no-progress` --
//! and holds the two stdouts against each other.
//!
//! Both runs must also render: a case whose bar emitted nothing would
//! compare two identical no-bar runs and pass on a broken renderer, so
//! each asserts its threshold lines reached stderr. The boot names the
//! reference cell and uses that cell's cap, so it walks the trajectory
//! the cell's anchor records over a real result stream.
//!
//! Needs an installed title: the contract is about a real boot's
//! output, and no synthetic input reaches the step loop.

#![allow(
    clippy::print_stderr,
    reason = "integration test: named not-installed skips are its only output channel"
)]

use std::process::Command;

use cellgov_compare::witnesses::TITLE_NOT_INSTALLED_SENTINEL;
use cellgov_compare::BootSummary;
use registry::{boot_anchor_path, titles, workspace_root, TitleUnderTest};

#[path = "common/registry.rs"]
mod registry;

/// One completed invocation.
struct Run {
    stdout: String,
    stderr: String,
}

/// Run `cellgov` with `args`, or `None` when the title is not
/// installed on this machine.
fn run(args: &[&str]) -> Option<Run> {
    let out = Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args(args)
        .current_dir(workspace_root())
        .output()
        .expect("spawn cellgov");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if stderr.contains(TITLE_NOT_INSTALLED_SENTINEL) {
        return None;
    }
    Some(Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr,
    })
}

fn tail(text: &str) -> String {
    text.lines().rev().take(12).collect::<Vec<_>>().join("\n")
}

fn assert_rendered(run: &Run, tag: &str, what: &str) {
    assert!(
        run.stderr.contains(&format!("[{tag}]")),
        "{what}: no [{tag}] threshold line reached stderr, so the comparison would \
         not have exercised the renderer. stderr tail:\n{}",
        tail(&run.stderr)
    );
}

fn assert_not_rendered(run: &Run, tag: &str, what: &str) {
    assert!(
        !run.stderr.contains(&format!("[{tag}]")),
        "{what}: --no-progress still rendered a [{tag}] line"
    );
}

/// A boot that dies inside the step loop has rendered already and
/// still prints no result line, so two such runs agree over nothing.
fn assert_result_stream(run: &Run, needle: &str, what: &str) {
    assert!(
        run.stdout.contains(needle),
        "{what}: stdout carries no {needle:?} line, so there was no result stream to \
         compare. stdout tail:\n{}",
        tail(&run.stdout)
    );
}

/// Registered titles, cheapest recorded trajectory first.
///
/// Every registered title carries a committed baseline
/// (`registry_structure` gates that), so an unreadable or malformed one
/// is a corpus defect rather than a reason to drop the title and
/// quietly re-order this list.
fn by_cost() -> Vec<(u64, TitleUnderTest)> {
    let mut out: Vec<(u64, TitleUnderTest)> = titles()
        .into_iter()
        .map(|t| {
            let path = boot_anchor_path(&t.content_id, &t.reference);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{}: read {}: {e}", t.short_name, path.display()));
            let summary: BootSummary = serde_json::from_str(&text).unwrap_or_else(|e| {
                panic!(
                    "{}: {} is not a BootSummary: {e}",
                    t.short_name,
                    path.display()
                )
            });
            (summary.steps, t)
        })
        .collect();
    out.sort_by_key(|(steps, _)| *steps);
    out
}

/// The first installed title's rendering run, with the argv that
/// produced it and the cap it used.
struct Baseline {
    args: Vec<String>,
    run: Run,
}

/// Run `build(title, cap)` against each title cheapest-first and keep
/// the first that is installed.
///
/// The successful invocation is the case's own rendering run, so
/// finding the title costs no extra boot.
fn first_installed(build: impl Fn(&str, &str) -> Vec<String>) -> Baseline {
    for (_, title) in by_cost() {
        // The reference cell's own cap. It bounds module_start too, so
        // a run under a smaller cap dies before the step loop.
        let cap = title.max_steps.to_string();
        let mut args = build(&title.short_name, &cap);
        // An unflagged selection refuses when the store holds several
        // firmwares or several game versions, so the boot names the
        // cell.
        args.push("--fw".to_string());
        args.push(title.reference.fw.clone());
        // A firmware-shipped title has no game-version axis, and the
        // composition refuses the flag for one.
        if let Some(v) = &title.reference.game_ver {
            args.push("--game-ver".to_string());
            args.push(v.clone());
        }
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        match run(&borrowed) {
            Some(run) => return Baseline { args, run },
            None => eprintln!("SKIP not installed: {}", title.short_name),
        }
    }
    panic!("no title in the registry is installed; the purity gate did not run");
}

/// Re-run a baseline's invocation with `extra` appended.
fn again_with(baseline: &Baseline, extra: &str) -> Run {
    let mut args: Vec<&str> = baseline.args.iter().map(String::as_str).collect();
    args.push(extra);
    run(&args).expect("the first invocation found this title installed")
}

/// Whether `tok` is a `Duration` the boot printed: digits, then a time
/// unit. Every boot stamps one on each module_start line, so both
/// cases below need the blanking, not just `boot run`.
fn is_duration(tok: &str) -> bool {
    let digits = tok.trim_end_matches(|c: char| !c.is_ascii_digit() && c != '.');
    let unit = &tok[digits.len()..];
    !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit() || c == '.')
        // Duration's own Debug spellings. The micro sign is escaped so
        // this file stays ASCII.
        && matches!(unit, "s" | "ms" | "ns" | "us" | "\u{b5}s")
}

/// `tok` with a wall-time measurement replaced by a fixed stand-in.
fn blank_token(tok: &str) -> String {
    for field in ["wall_ns=", "steps_per_sec="] {
        if tok.starts_with(field) {
            return format!("{field}<measured>");
        }
    }
    if is_duration(tok) {
        return "<measured>".to_string();
    }
    tok.to_string()
}

/// Blank every token that measures wall time, line structure kept.
///
/// Two `BENCH_RESULT` fields and the per-module_start durations are
/// the only things a boot prints that its own determinism does not
/// fix. Blanking exactly those leaves the comparison exact over the
/// rest, which is the whole result stream. The mapping is per line, so
/// a stray newline on the result stream is a difference rather than
/// whitespace the token split absorbs.
fn blank_measurements(stdout: &str) -> String {
    stdout
        .lines()
        .map(|line| {
            line.split_whitespace()
                .map(blank_token)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_bench_bar_never_changes_what_stdout_says() {
    let with_bar = first_installed(|title, cap| {
        vec![
            "boot".into(),
            "bench-once".into(),
            "--title".into(),
            title.into(),
            "--max-steps".into(),
            cap.into(),
        ]
    });
    let without_bar = again_with(&with_bar, "--no-progress");

    assert_rendered(&with_bar.run, "bench", "boot bench-once");
    assert_not_rendered(&without_bar, "bench", "boot bench-once");
    assert_result_stream(
        &with_bar.run,
        "BENCH_RESULT ",
        "boot bench-once with the bar",
    );
    assert_result_stream(
        &without_bar,
        "BENCH_RESULT ",
        "boot bench-once --no-progress",
    );
    assert_eq!(
        blank_measurements(&with_bar.run.stdout),
        blank_measurements(&without_bar.stdout),
        "the bar moved boot bench-once's result stream"
    );
    assert!(
        !with_bar.run.stdout.contains("[bench]"),
        "a threshold line landed on the result stream"
    );
}

/// The step loop's liveness reporting is what used to interleave with
/// guest TTY output on stdout, so this is the case the contract was
/// written for.
#[test]
fn the_boot_run_bar_never_changes_what_stdout_says() {
    let with_bar = first_installed(|title, cap| {
        vec![
            "boot".into(),
            "run".into(),
            "--title".into(),
            title.into(),
            "--max-steps".into(),
            cap.into(),
        ]
    });
    let without_bar = again_with(&with_bar, "--no-progress");

    assert_rendered(&with_bar.run, "boot", "boot run");
    assert_not_rendered(&without_bar, "boot", "boot run");
    assert_result_stream(&with_bar.run, "outcome: ", "boot run with the bar");
    assert_result_stream(&without_bar, "outcome: ", "boot run --no-progress");
    assert_eq!(
        blank_measurements(&with_bar.run.stdout),
        blank_measurements(&without_bar.stdout),
        "the bar moved boot run's result stream"
    );
    assert!(
        !with_bar.run.stdout.contains("[boot]"),
        "a threshold line landed on the result stream"
    );
}
