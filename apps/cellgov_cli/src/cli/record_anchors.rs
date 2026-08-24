//! `record-anchors`: re-measure a title's boot and rewrite its
//! committed baseline.
//!
//! The witness suite asserts against `boot_summary.json`; this is the
//! only thing that writes it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use cellgov_compare::boot_history::{self, BootHistoryEntry};
use cellgov_compare::runner_cellgov::BootOutcome;
use cellgov_compare::witness_parse::parse_witness_lines;
use cellgov_compare::witnesses::{record, BOOT_STARTED_SENTINEL, TITLE_NOT_INSTALLED_SENTINEL};
use cellgov_compare::BootSummary;

use crate::cli::exit::die;
use crate::cli::title::DEFAULT_TITLE_REGISTRY_DIR;
use crate::game::manifest::TitleRegistry;

use crate::paths::{anchor_max_steps, boot_anchor_path, history_path, workspace_root};

use crate::cli::args::{find_flag_value, has_bool_flag};

struct Entry {
    short_name: String,
    content_id: String,
    max_steps: u64,
}

fn read_registry(dir: &Path) -> Vec<Entry> {
    let registry = TitleRegistry::scan_dir(dir)
        .unwrap_or_else(|e| die(&format!("scan registry {}: {e}", dir.display())));
    let mut out: Vec<Entry> = registry
        .iter()
        .map(|m| Entry {
            short_name: m.short_name.clone(),
            content_id: m.content_id.clone(),
            max_steps: anchor_max_steps(m),
        })
        .collect();
    out.sort_by(|a, b| a.short_name.cmp(&b.short_name));
    out
}

/// Boot one title; `None` when its dump is not installed (the boot
/// printed the not-installed marker). Any other failure dies: past
/// the boot-inputs sentinel, a broken run must never look like a
/// skip.
fn measure(entry: &Entry) -> Option<(BTreeMap<String, u64>, u64, String)> {
    let exe = std::env::current_exe().unwrap_or_else(|e| die(&format!("current_exe: {e}")));
    let output = Command::new(exe)
        .arg("bench-boot-once")
        .arg("--title")
        .arg(&entry.short_name)
        .arg("--max-steps")
        .arg(entry.max_steps.to_string())
        .current_dir(workspace_root())
        .output()
        .unwrap_or_else(|e| die(&format!("spawn bench-boot-once: {e}")));

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        if stderr.contains(TITLE_NOT_INSTALLED_SENTINEL) {
            return None;
        }
        let phase = if stderr.contains(BOOT_STARTED_SENTINEL) {
            "boot started then failed"
        } else {
            "boot inputs failed to resolve (dump present but unusable?)"
        };
        die(&format!(
            "{}: {phase}; refusing to record a broken run. stderr tail:\n{}",
            entry.short_name,
            stderr.lines().rev().take(8).collect::<Vec<_>>().join("\n")
        ));
    }

    let witnesses = parse_witness_lines(&stderr).unwrap_or_else(|errs| {
        let lines: Vec<String> = errs.iter().map(ToString::to_string).collect();
        die(&format!(
            "{}: malformed witness lines:\n  {}",
            entry.short_name,
            lines.join("\n  ")
        ))
    });

    let result = stdout
        .lines()
        .find(|l| l.starts_with("BENCH_RESULT"))
        .unwrap_or_else(|| die(&format!("{}: no BENCH_RESULT line", entry.short_name)));
    let mut steps = None;
    let mut outcome = None;
    for tok in result.split_whitespace() {
        if let Some(v) = tok.strip_prefix("steps=") {
            steps = v.parse::<u64>().ok();
        } else if let Some(v) = tok.strip_prefix("outcome=") {
            outcome = Some(v.to_string());
        }
    }
    let steps =
        steps.unwrap_or_else(|| die(&format!("{}: BENCH_RESULT has no steps=", entry.short_name)));
    let outcome = outcome.unwrap_or_else(|| {
        die(&format!(
            "{}: BENCH_RESULT has no outcome=",
            entry.short_name
        ))
    });
    Some((witnesses.values, steps, outcome))
}

/// Read and parse the existing history, dying on any error other than
/// a missing file. A read failure must not be mistaken for an empty
/// history -- that would silently replace the append-only record.
fn read_history(path: &Path) -> String {
    match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => die(&format!(
            "read {}: {e}; refusing to treat an unreadable history as empty",
            path.display()
        )),
    }
}

/// The committed anchor this run updates.
///
/// The three ways to not get one are kept apart: only an absent file
/// means "never recorded". An unreadable or unparseable file names
/// itself, because the anchor carries the checkpoint and budget this
/// command does not measure -- rewriting it from scratch would drop
/// curated fields that are still on disk.
fn read_previous_anchor(short_name: &str, path: &Path) -> BootSummary {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => die(&format!(
            "{short_name}: no existing {} to update. The baseline carries checkpoint and \
             budget, which this command does not measure; create it first with \
             run-game --save-boot-summary.",
            path.display()
        )),
        Err(e) => die(&format!(
            "{short_name}: read {}: {e}; refusing to treat an unreadable anchor as absent",
            path.display()
        )),
    };
    serde_json::from_str(&text).unwrap_or_else(|e| {
        die(&format!(
            "{short_name}: parse {}: {e}; the anchor exists but is malformed. Repair it \
             rather than letting this run recreate it without its checkpoint and budget.",
            path.display()
        ))
    })
}

/// Rewrite one title's baseline, preserving any hand-promoted witness
/// class. Returns `false` when the title is not installed: `--all`
/// skips it by name, `--title` treats it as an error.
fn record_one(entry: &Entry, strict: bool) -> bool {
    let Some((witnesses, steps, outcome)) = measure(entry) else {
        if strict {
            die(&format!(
                "{}: the title's dump is not installed",
                entry.short_name
            ));
        }
        println!(
            "{}: skipped -- not installed on this machine",
            entry.short_name
        );
        return false;
    };

    let path = boot_anchor_path(&workspace_root(), &entry.content_id);
    let mut summary = read_previous_anchor(&entry.short_name, &path);

    // History is parsed BEFORE the baseline is written: a malformed
    // history line must abort while the anchor is still untouched,
    // never leave a moved anchor with no history entry.
    let hist_path = history_path(&workspace_root(), &entry.content_id);
    let existing_history = read_history(&hist_path);
    let history_entries = boot_history::parse(&existing_history)
        .unwrap_or_else(|e| die(&format!("parse {}: {e}", hist_path.display())));
    let history_entry = BootHistoryEntry::new_if_changed(
        history_entries.last(),
        steps,
        &outcome,
        witnesses.clone(),
    );

    let before = summary.witnesses.clone();
    summary.steps = steps;
    summary.outcome = BootOutcome::from_str(&outcome).unwrap_or_else(|e| {
        die(&format!(
            "{}: BENCH_RESULT outcome {outcome:?} did not parse: {e}",
            entry.short_name
        ))
    });
    summary.host_invariant_breaks = witnesses.get("host_invariant_breaks").copied().unwrap_or(0);
    summary.witnesses = record(Some(&before), &witnesses);
    summary.validate().unwrap_or_else(|e| {
        die(&format!(
            "{}: recorded summary is invalid: {e}",
            entry.short_name
        ))
    });

    let json = serde_json::to_string_pretty(&summary)
        .unwrap_or_else(|e| die(&format!("serialize {}: {e}", path.display())));
    std::fs::write(&path, json + "\n")
        .unwrap_or_else(|e| die(&format!("write {}: {e}", path.display())));

    match history_entry {
        None => println!(
            "{}: unchanged ({outcome}, {steps} steps, {} witnesses)",
            entry.short_name,
            summary.witnesses.len()
        ),
        Some(entry_line) => {
            let line = boot_history::render_line(&entry_line)
                .unwrap_or_else(|e| die(&format!("serialize history entry: {e}")));
            let mut text = existing_history;
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&line);
            std::fs::write(&hist_path, text)
                .unwrap_or_else(|e| die(&format!("write {}: {e}", hist_path.display())));
            println!(
                "{}: moved ({outcome}, {steps} steps) -- {}",
                entry.short_name,
                entry_line.changed.join(", ")
            );
        }
    }
    true
}

/// Refuse a `--registry` the spawned measurement cannot honour.
///
/// `measure` re-enters the binary as `bench-boot-once --title <name>`,
/// and that path resolves the name against the compiled-in registry
/// directory. A `--registry` pointing elsewhere would enumerate one
/// set of manifests, boot the same-named title from another, and then
/// write the anchor under the first manifest's content id.
fn reject_unforwardable_registry(registry: &Path, default: &Path) {
    let same = match (registry.canonicalize(), default.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => registry == default,
    };
    if !same {
        die(&format!(
            "record-anchors: --registry {} is not the registry the measurement reads. \
             The boot is re-entered as `bench-boot-once --title <name>`, which resolves \
             names against {}, so the anchor would be measured from one manifest and \
             filed under another. Point --registry at {} or drop it.",
            registry.display(),
            default.display(),
            default.display(),
        ));
    }
}

pub(crate) fn run(args: &[String]) {
    let default_registry = workspace_root().join(DEFAULT_TITLE_REGISTRY_DIR);
    // The shared parsers, not a local scan: they refuse `--flag=value`
    // and a duplicate, both of which a bare `position(|a| a == name)`
    // reads as absent.
    let registry = match find_flag_value(args, "--registry") {
        Some(p) => {
            let given = PathBuf::from(p);
            reject_unforwardable_registry(&given, &default_registry);
            given
        }
        None => default_registry,
    };
    let all = has_bool_flag(args, "--all");
    let one = find_flag_value(args, "--title");
    if all == one.is_some() {
        die("record-anchors requires exactly one of --all or --title <name>");
    }

    let entries = read_registry(&registry);
    if entries.is_empty() {
        die(&format!("no title manifests under {}", registry.display()));
    }

    let selected: Vec<&Entry> = match one.as_deref() {
        Some(name) => {
            let hit = entries.iter().find(|e| e.short_name == name);
            let Some(hit) = hit else {
                let known: Vec<&str> = entries.iter().map(|e| e.short_name.as_str()).collect();
                die(&format!(
                    "unknown title {name:?}; registry has: {}",
                    known.join(", ")
                ));
            };
            vec![hit]
        }
        None => entries.iter().collect(),
    };

    let strict = one.is_some();
    let mut recorded = 0usize;
    let total = selected.len();
    for entry in selected {
        if record_one(entry, strict) {
            recorded += 1;
        }
    }
    // --all over a machine with zero installed titles must not exit 0
    // having recorded nothing.
    if recorded == 0 {
        die(&format!(
            "none of the {total} registered title(s) is installed; nothing recorded"
        ));
    }
}
