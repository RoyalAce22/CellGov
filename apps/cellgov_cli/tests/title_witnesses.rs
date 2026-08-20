//! Every installed title in the registry must reproduce its recorded
//! baseline.
//!
//! Titles come from `docs/title_manifests/`; expectations come from
//! each title's committed `boot_summary.json`. Adding a title needs no
//! change here -- drop in a manifest, record it, commit the baseline.
//!
//! The registry is shared but installs vary per operator, so a title
//! whose boot prints the not-installed marker skips by name; at least
//! one title must boot or the suite fails, keeping "green means
//! something ran" true under the `title-corpus` feature. Any other
//! failing boot -- including one that dies before its inputs resolve,
//! e.g. a present-but-undecryptable dump -- is a suite failure.
//! Re-record with:
//!
//! ```text
//! cargo run --release -p cellgov_cli -- record-anchors --all
//! ```

#![allow(
    clippy::print_stderr,
    reason = "integration test: named not-installed skips are its only output channel"
)]

#[path = "common/registry.rs"]
mod registry;

use std::process::Command;

use cellgov_compare::witness_parse::{parse_witness_lines, ParsedWitnesses};
use cellgov_compare::witnesses::{check_all, unrecorded, TITLE_NOT_INSTALLED_SENTINEL};
use cellgov_compare::BootSummary;
use registry::{baseline_path, titles, workspace_root, TitleUnderTest};

struct Observed {
    witnesses: ParsedWitnesses,
    steps: u64,
    outcome: String,
}

/// How a boot attempt ended, separating "this operator does not have
/// the title" from "the title booted wrong".
enum Boot {
    NotInstalled,
    Ran(Observed),
    Failed(String),
}

fn boot(title: &TitleUnderTest) -> Boot {
    let output = Command::new(env!("CARGO_BIN_EXE_cellgov_cli"))
        .arg("bench-boot-once")
        .arg("--title")
        .arg(&title.short_name)
        .arg("--max-steps")
        .arg(title.max_steps.to_string())
        .current_dir(workspace_root())
        .output()
        .expect("spawn cellgov_cli bench-boot-once");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

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

    let witnesses = match parse_witness_lines(&stderr) {
        Ok(w) => w,
        Err(errs) => {
            let lines: Vec<String> = errs.iter().map(ToString::to_string).collect();
            return Boot::Failed(format!(
                "malformed witness lines:\n  {}",
                lines.join("\n  ")
            ));
        }
    };

    let Some(result) = stdout.lines().find(|l| l.starts_with("BENCH_RESULT")) else {
        return Boot::Failed("no BENCH_RESULT line on stdout".to_string());
    };
    let mut steps = None;
    let mut outcome = None;
    for tok in result.split_whitespace() {
        if let Some(v) = tok.strip_prefix("steps=") {
            steps = v.parse::<u64>().ok();
        } else if let Some(v) = tok.strip_prefix("outcome=") {
            outcome = Some(v.to_string());
        }
    }
    let (Some(steps), Some(outcome)) = (steps, outcome) else {
        return Boot::Failed(format!("BENCH_RESULT missing steps=/outcome=: {result}"));
    };
    Boot::Ran(Observed {
        witnesses,
        steps,
        outcome,
    })
}

/// Compare one installed title against its baseline; `None` means the
/// title is not installed on this machine.
///
/// The boot runs before the baseline is read: a missing baseline only
/// matters for a title this operator can actually record.
fn check_title(title: &TitleUnderTest) -> Option<Vec<String>> {
    let observed = match boot(title) {
        Boot::NotInstalled => return None,
        Boot::Failed(e) => return Some(vec![format!("{}: {e}", title.short_name)]),
        Boot::Ran(o) => o,
    };

    let path = baseline_path(&title.content_id);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Some(vec![format!(
            "{}: installed but no baseline at {}. Record it with:\n    \
             cargo run --release -p cellgov_cli -- record-anchors --title {}",
            title.short_name,
            path.display(),
            title.short_name
        )]);
    };
    let baseline: BootSummary = match serde_json::from_str(&text) {
        Ok(b) => b,
        Err(e) => {
            return Some(vec![format!(
                "{}: {} failed to parse: {e}",
                title.short_name,
                path.display()
            )])
        }
    };

    let mut failures = Vec::new();
    if observed.steps != baseline.steps {
        failures.push(format!(
            "{}: steps {} != recorded {}",
            title.short_name, observed.steps, baseline.steps
        ));
    }
    // Display, not Debug: BENCH_RESULT prints the Display form, and
    // the two disagree for PcReached, whose Debug renders the address
    // in decimal.
    let recorded_outcome = baseline.outcome.to_string();
    if observed.outcome != recorded_outcome {
        failures.push(format!(
            "{}: outcome {} != recorded {recorded_outcome}",
            title.short_name, observed.outcome
        ));
    }
    if baseline.witnesses.is_empty() {
        failures.push(format!(
            "{}: baseline records no witnesses. Re-record with:\n    \
             cargo run --release -p cellgov_cli -- record-anchors --title {}",
            title.short_name, title.short_name
        ));
        return Some(failures);
    }
    for failure in check_all(&baseline.witnesses, &observed.witnesses) {
        failures.push(format!("{}: {failure}", title.short_name));
    }
    for name in unrecorded(&baseline.witnesses, &observed.witnesses.values) {
        failures.push(format!(
            "{}: witness {name} is emitted but not recorded -- re-record the baseline",
            title.short_name
        ));
    }
    Some(failures)
}

#[test]
fn every_installed_title_matches_its_recorded_baseline() {
    let titles = titles();
    let mut failures: Vec<String> = Vec::new();
    let mut skipped: Vec<&str> = Vec::new();
    let mut checked = 0usize;
    for title in &titles {
        match check_title(title) {
            None => {
                eprintln!(
                    "{}: skipped -- not installed on this machine",
                    title.short_name
                );
                skipped.push(&title.short_name);
            }
            Some(f) => {
                checked += 1;
                failures.extend(f);
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} failure(s) across {checked} installed title(s):\n\n{}\n",
        failures.len(),
        failures.join("\n")
    );
    // Anti-vacuity floor: the feature declares a corpus, so a run
    // that booted nothing must not report green.
    assert!(
        checked > 0,
        "title-corpus is enabled but none of the {} registered title(s) is \
         installed (skipped: {}). Install at least one, or run without the \
         feature.",
        titles.len(),
        skipped.join(", ")
    );
}
