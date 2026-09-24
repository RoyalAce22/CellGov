//! Every installed title in the registry must reproduce its gated
//! cells' recorded baselines.
//!
//! Titles come from `title_manifests/`. Expectations come from the
//! anchor of each gated cell: a game title's floor times its base
//! install, or each declared firmware of a firmware-shipped title.
//! Adding a title needs no change here -- drop in a manifest, record
//! it, commit the baseline.
//!
//! The registry is shared but installs vary per operator, so two kinds
//! of cell skip by name:
//!
//! - a title whose boot prints the not-installed marker, and
//! - a cell the registry declares `pending`.
//!
//! At least one cell must boot or the suite fails; under the
//! `installed-title-tests` feature, green means something ran. Any other failing
//! boot -- including one that dies before its inputs resolve, e.g. a
//! present-but-undecryptable dump -- is a suite failure. Re-record with:
//!
//! ```text
//! cargo run --release -p cellgov_cli -- dev record-anchors --all
//! ```

#![allow(
    clippy::print_stderr,
    reason = "integration test: named not-installed skips are its only output channel"
)]

#[path = "common/registry.rs"]
mod registry;

use std::process::Command;

use cellgov_boot::manifest::{CellKey, TitleRegistry};
use cellgov_compare::bench::{hold_against_anchor, load_anchor, parse_bench_result, MeasuredRun};
use cellgov_compare::witnesses::TITLE_NOT_INSTALLED_SENTINEL;
use cellgov_compare::CheckpointKind;
use registry::{boot_anchor_path, firmware_exec_titles, titles, workspace_root, TitleUnderTest};

/// How a boot attempt ended, separating "this operator does not have
/// the title" from "the title booted wrong".
enum Boot {
    NotInstalled,
    /// The child's stdout and stderr.
    Ran(String, String),
    Failed(String),
}

fn boot(title: &TitleUnderTest) -> Boot {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cellgov"));
    cmd.args(["boot", "bench-once"])
        .arg("--title")
        .arg(&title.short_name)
        .arg("--fw")
        .arg(&title.reference.fw)
        .arg("--max-steps")
        .arg(title.max_steps.to_string());
    // A firmware-shipped title has no game-version axis, and the
    // composition refuses the flag for one.
    if let Some(v) = &title.reference.game_ver {
        cmd.arg("--game-ver").arg(v);
    }
    let output = cmd
        .current_dir(workspace_root())
        .output()
        .expect("spawn cellgov boot bench-once");

    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();

    if !output.status.success() {
        // Only the explicit marker is a skip: a dump that exists but
        // fails to decrypt or parse must fail the suite, not vanish.
        if stderr.contains(TITLE_NOT_INSTALLED_SENTINEL) {
            return Boot::NotInstalled;
        }
        let tail: Vec<&str> = stderr.lines().rev().take(30).collect();
        return Boot::Failed(format!(
            "boot failed (exit {:?}). Last stderr lines (newest first):\n  {}",
            output.status.code(),
            tail.join("\n  ")
        ));
    }
    Boot::Ran(stdout, stderr)
}

/// The stop condition the registry declares for the cell `title` boots,
/// which is the one `boot bench-once` ran at: the suite passes no
/// `--checkpoint`.
fn declared_checkpoint(title: &TitleUnderTest) -> CheckpointKind {
    let registry = TitleRegistry::scan_dir(&workspace_root().join("title_manifests"))
        .expect("the committed registry loads");
    let manifest = registry
        .by_short_name(&title.short_name)
        .unwrap_or_else(|| panic!("{} is not in the registry", title.short_name));
    let key = CellKey {
        fw: title.reference.fw.clone(),
        game_ver: title.reference.game_ver.clone(),
    };
    manifest.cell_checkpoint(manifest.cell(&key)).kind()
}

/// Compare one installed title against its baseline; `None` means the
/// title is not installed on this machine.
///
/// The boot runs before the baseline is read: a missing baseline only
/// matters for a title this operator can actually record. The
/// comparison is the one `boot bench` gates with.
fn check_title(title: &TitleUnderTest) -> Option<Vec<String>> {
    // One short name can gate several cells (the system software gates
    // one per declared firmware), so a failure names the cell.
    let who = format!("{} ({})", title.short_name, title.reference.label());
    let (stdout, stderr) = match boot(title) {
        Boot::NotInstalled => return None,
        Boot::Failed(e) => return Some(vec![format!("{who}: {e}")]),
        Boot::Ran(stdout, stderr) => (stdout, stderr),
    };
    let result = match parse_bench_result(&stdout) {
        Ok(parsed) => parsed.result,
        Err(e) => return Some(vec![format!("{who}: {e}")]),
    };

    let path = boot_anchor_path(&title.content_id, &title.reference);
    let baseline = match load_anchor(&path) {
        Ok(Some(b)) => b,
        Ok(None) => {
            return Some(vec![format!(
                "{who}: installed but no baseline at {}. Record it with:\n    \
                 cargo run --release -p cellgov_cli -- dev record-anchors --title {} --fw {}",
                path.display(),
                title.short_name,
                title.reference.fw
            )])
        }
        Err(e) => return Some(vec![format!("{who}: {e}")]),
    };

    let run = MeasuredRun {
        checkpoint: declared_checkpoint(title),
        steps: result.steps as u64,
        budget: result.budget,
        outcome: result.outcome,
        stderr: &stderr,
    };
    let mut failures: Vec<String> = hold_against_anchor(&baseline, &run)
        .into_iter()
        .map(|failure| format!("{who}: {failure}"))
        .collect();
    if !failures.is_empty() {
        failures.push(format!(
            "{who}: if the move is intended, re-record it with:\n    \
             cargo run --release -p cellgov_cli -- dev record-anchors --title {} --fw {}",
            title.short_name, title.reference.fw
        ));
    }
    Some(failures)
}

#[test]
fn every_installed_title_matches_its_recorded_baseline() {
    let titles: Vec<TitleUnderTest> = titles().into_iter().chain(firmware_exec_titles()).collect();
    let mut failures: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for title in &titles {
        // Something outside the registry stops a pending cell, and the
        // structure gate refuses an anchor on it, so there is nothing
        // here to hold a boot against.
        if let Some(why) = &title.reference.pending {
            eprintln!(
                "{} ({}): skipped -- declared pending ({why})",
                title.short_name,
                title.reference.label()
            );
            skipped.push(format!("{} {}", title.short_name, title.reference.label()));
            continue;
        }
        match check_title(title) {
            None => {
                eprintln!(
                    "{} ({}): skipped -- not installed on this machine",
                    title.short_name,
                    title.reference.label()
                );
                skipped.push(format!("{} {}", title.short_name, title.reference.label()));
            }
            Some(f) => {
                checked += 1;
                failures.extend(f);
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} failure(s) across {checked} installed cell(s):\n\n{}\n",
        failures.len(),
        failures.join("\n")
    );
    // Anti-vacuity floor: the feature declares installed titles, so a run
    // that booted nothing must not report green.
    assert!(
        checked > 0,
        "installed-title-tests is enabled but none of the {} gated cell(s) is \
         installed (skipped: {}). Install at least one, or run without the \
         feature.",
        titles.len(),
        skipped.join(", ")
    );
}
