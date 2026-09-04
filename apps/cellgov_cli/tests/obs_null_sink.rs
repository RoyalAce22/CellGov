//! Observability inertness gate: a boot that wipes the host's
//! instruments after every committed step must produce the same
//! binary state trace as one that records them.
//!
//! Two `boot run` runs per title -- one normal, one under
//! `CELLGOV_OBS_NULL_SINK=1` -- with `--save-state-trace`; the
//! trace streams must be byte-identical. Any drift means an
//! instrument steered execution, which is the boundary
//! `Lv2Observability` exists to enforce.
//!
//! Per-instruction state hashing makes trace size and wall time
//! scale with the trajectory, so the gate boots only the installed
//! title with the smallest committed baseline, skipping titles this
//! operator does not have. Zero installed titles fails the suite.

#![allow(
    clippy::print_stderr,
    reason = "integration test: named not-installed skips are its only output channel"
)]

#[path = "common/registry.rs"]
mod registry;

use std::path::PathBuf;
use std::process::Command;

use cellgov_compare::witnesses::TITLE_NOT_INSTALLED_SENTINEL;
use cellgov_compare::BootSummary;
use cellgov_testkit::scratch::scratch_labeled;
use registry::{boot_anchor_path, titles, workspace_root, TitleUnderTest};

enum Run {
    NotInstalled,
    /// Trace bytes plus the process exit code; `boot run` maps the
    /// boot outcome to its exit status, so the pair must agree
    /// between the two runs.
    Trace(Vec<u8>, Option<i32>),
}

fn boot_with_trace(title: &TitleUnderTest, trace_path: &PathBuf, null_sink: bool) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cellgov"));
    cmd.args(["boot", "run"])
        .arg("--title")
        .arg(&title.short_name)
        // The cap and the anchor are the reference cell's, so the boot
        // names that cell. An unflagged selection refuses when the
        // store holds several firmwares or several game versions.
        .arg("--fw")
        .arg(&title.reference.fw)
        .arg("--max-steps")
        .arg(title.max_steps.to_string())
        .arg("--save-state-trace")
        .arg(trace_path)
        .current_dir(workspace_root());
    // A firmware-shipped title has no game-version axis, and the
    // composition refuses the flag for one.
    if let Some(v) = &title.reference.game_ver {
        cmd.arg("--game-ver").arg(v);
    }
    // Scrub rather than merely not set: `Command` inherits the parent
    // environment, so an exported CELLGOV_OBS_NULL_SINK would turn
    // both runs into nulled runs and make the comparison vacuous.
    if null_sink {
        cmd.env("CELLGOV_OBS_NULL_SINK", "1");
    } else {
        cmd.env_remove("CELLGOV_OBS_NULL_SINK");
    }
    let output = cmd.output().expect("spawn cellgov boot run");
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains(TITLE_NOT_INSTALLED_SENTINEL) {
        return Run::NotInstalled;
    }
    // The trace file is the boot's own completion witness: boot run
    // writes it after the step loop regardless of outcome, so a boot
    // that died before running leaves no file.
    let bytes = std::fs::read(trace_path).unwrap_or_else(|e| {
        let tail: Vec<&str> = stderr.lines().rev().take(20).collect();
        panic!(
            "{} (null_sink={null_sink}): no state trace at {} ({e}); exit {:?}. \
             Last stderr lines (newest first):\n  {}",
            title.short_name,
            trace_path.display(),
            output.status.code(),
            tail.join("\n  ")
        )
    });
    assert!(
        !bytes.is_empty(),
        "{}: state trace is empty; the comparison would be vacuous",
        title.short_name
    );
    Run::Trace(bytes, output.status.code())
}

#[test]
fn observability_is_inert_wiping_it_every_step_leaves_the_state_trace_byte_identical() {
    let scratch = scratch_labeled("obs_null_sink");

    // Every registered title carries a committed baseline
    // (`registry_structure` gates that), so an unreadable or malformed
    // one is a corpus defect. Dropping it would quietly re-order the
    // cheapest-first selection below and boot a different title than
    // the one this gate is sized for.
    let mut by_cost: Vec<(u64, TitleUnderTest)> = titles()
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
    by_cost.sort_by_key(|(steps, _)| *steps);

    let mut ran = 0usize;
    for (_, title) in by_cost {
        let recording_path = scratch.join(format!("{}_recording.trace", title.short_name));
        let (recording, recording_exit) = match boot_with_trace(&title, &recording_path, false) {
            Run::NotInstalled => {
                eprintln!("SKIP not installed: {}", title.short_name);
                continue;
            }
            Run::Trace(b, code) => (b, code),
        };
        let nulled_path = scratch.join(format!("{}_nulled.trace", title.short_name));
        let (nulled, nulled_exit) = match boot_with_trace(&title, &nulled_path, true) {
            Run::NotInstalled => panic!(
                "{}: installed for the recording run but not the nulled run",
                title.short_name
            ),
            Run::Trace(b, code) => (b, code),
        };
        assert_eq!(
            recording_exit, nulled_exit,
            "{}: boot outcome (exit code) diverged under the null sink",
            title.short_name
        );
        assert_eq!(
            recording.len(),
            nulled.len(),
            "{}: state-trace length diverged under the null sink",
            title.short_name
        );
        assert!(
            recording == nulled,
            "{}: state-trace bytes diverged under the null sink; an instrument steered execution",
            title.short_name
        );
        ran += 1;
        break;
    }
    assert!(
        ran > 0,
        "no title in the registry is installed; the inertness gate did not run"
    );
}
