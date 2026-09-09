//! `dev record-anchors`: re-measure the cells a title declares and
//! rewrite their committed anchors.
//!
//! The witness suite asserts against each cell's `boot_summary.json`;
//! this is the only thing that writes one. It records the cells the
//! registry declares -- the one `[title] system_ver` derives and every
//! `[[bench.matrix]]` row -- and refuses a cell it does not.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::str::FromStr;

use cellgov_compare::boot_history::{self, BootHistoryEntry};
use cellgov_compare::runner_cellgov::BootOutcome;
use cellgov_compare::witness_parse::parse_witness_lines;
use cellgov_compare::witnesses::{record, BOOT_STARTED_SENTINEL, TITLE_NOT_INSTALLED_SENTINEL};
use cellgov_compare::{BootSummary, RunIdentity, RUN_IDENTITY_SENTINEL};
use cellgov_terminal::caps::RenderFlags;
use cellgov_terminal::progress::{ProgressBar, ProgressSink as _};
use cellgov_time::Budget;

use crate::cli::exit::die;
use crate::cli::title::DEFAULT_TITLE_REGISTRY_DIR;
use crate::game::manifest::{
    CellKey, CheckpointTrigger, TitleManifest, TitleRegistry, BASE_GAME_VER,
};

use crate::paths::{
    boot_anchor_path, cell_checkpoint, cell_max_steps, checkpoint_kind, history_path,
    workspace_root,
};
use crate::progress::RECORD_ANCHORS_TASK;

use crate::cli::parse::RecordAnchorsArgs;

/// One declared cell to re-measure.
struct Job {
    short_name: String,
    content_id: String,
    cell: CellKey,
    max_steps: u64,
    checkpoint: CheckpointTrigger,
    /// The registry's reason this cell has no measurement yet.
    pending: Option<String>,
}

impl Job {
    /// How a report and a refusal name this job.
    fn label(&self) -> String {
        format!("{} {}", self.short_name, self.cell.label())
    }
}

/// Every cell `title` declares, in declaration order.
fn jobs_for(title: &TitleManifest) -> Vec<Job> {
    title
        .matrix
        .iter()
        .map(|cell| Job {
            short_name: title.short_name.clone(),
            content_id: title.content_id.clone(),
            cell: cell.key.clone(),
            max_steps: cell_max_steps(title, Some(cell)),
            checkpoint: cell_checkpoint(title, Some(cell)),
            pending: cell.pending.clone(),
        })
        .collect()
}

/// Separate the cells the registry declares `pending`, unless `--fw`
/// or `--game-ver` narrowed the selection.
///
/// Something outside the registry stops a pending cell. A sweep that
/// measures it dies at that boot and takes every other cell of the
/// title with it. A named selection asks for the measurement
/// regardless; once the anchor exists, the structure gate says to drop
/// the marker.
fn skip_pending(jobs: Vec<Job>, narrowed: bool) -> (Vec<Job>, Vec<Job>) {
    jobs.into_iter()
        .partition(|j| narrowed || j.pending.is_none())
}

fn read_registry(dir: &Path) -> Vec<TitleManifest> {
    let registry = TitleRegistry::scan_dir(dir)
        .unwrap_or_else(|e| die(&format!("scan registry {}: {e}", dir.display())));
    let mut out: Vec<TitleManifest> = registry.iter().cloned().collect();
    out.sort_by(|a, b| a.short_name.cmp(&b.short_name));
    out
}

struct Measurement {
    witnesses: BTreeMap<String, u64>,
    steps: u64,
    budget: Budget,
    outcome: String,
    identity: RunIdentity,
}

/// One `u64` field of the `BENCH_RESULT` line.
fn parse_result_field(job: &Job, name: &str, value: &str) -> u64 {
    value.parse().unwrap_or_else(|e| {
        die(&format!(
            "{}: BENCH_RESULT {name}={value:?} did not parse: {e}",
            job.label()
        ))
    })
}

/// The `version` a run's identity carries for a cell's game-version
/// axis.
///
/// The composition spells a selected update `update:<version>` and the
/// base install `base`. A cell's `game_ver` and the identity's version
/// are two spellings of one value.
fn identity_game_version(game_ver: &str) -> String {
    if game_ver == BASE_GAME_VER {
        game_ver.to_string()
    } else {
        format!("update:{game_ver}")
    }
}

/// Every way the run's own identity contradicts the cell its result
/// would be filed under; empty when the two name one triple.
///
/// The child re-resolves its composition, and not every flag reaches
/// that resolution. A boot asked for no firmware never consults `--fw`;
/// it composes a firmware-free run whatever the flag named
/// (`resolve_composition` in `boot_cmd`). The identity line reports what
/// the run composed, so it decides which cell may receive the result.
fn cell_disagreements(identity: &RunIdentity, cell: &CellKey) -> Vec<String> {
    let mut out = Vec::new();
    match &identity.firmware {
        Some(f) if f.version == cell.fw => {}
        Some(f) => out.push(format!(
            "composed firmware {} rather than {}",
            f.version, cell.fw
        )),
        None => out.push(format!(
            "composed no managed firmware rather than firmware {}",
            cell.fw
        )),
    }
    let want = cell.game_ver.as_deref().map(identity_game_version);
    let got = identity.game.as_ref().map(|g| g.version.clone());
    if want != got {
        out.push(format!(
            "composed game version {} rather than {}",
            got.as_deref().unwrap_or("(none)"),
            want.as_deref().unwrap_or("(none)")
        ));
    }
    out
}

/// Boot one cell; `None` when its dump is not installed (the boot
/// printed the not-installed marker). Any other failure dies: past
/// the boot-inputs sentinel, a broken run must never look like a
/// skip.
fn measure(job: &Job) -> Option<Measurement> {
    let exe = std::env::current_exe().unwrap_or_else(|e| die(&format!("current_exe: {e}")));
    let mut cmd = Command::new(exe);
    cmd.arg("boot")
        .arg("bench-once")
        // Both of the child's streams are captured and parsed here, so
        // a child bar would render into a pipe rather than a terminal.
        .arg("--no-progress")
        .arg("--title")
        .arg(&job.short_name)
        .arg("--fw")
        .arg(&job.cell.fw)
        .arg("--max-steps")
        .arg(job.max_steps.to_string())
        .arg("--checkpoint")
        .arg(job.checkpoint.as_cli_str());
    // A firmware-shipped title has no game-version axis, and the
    // composition refuses the flag for one.
    if let Some(v) = &job.cell.game_ver {
        cmd.arg("--game-ver").arg(v);
    }
    let output = cmd
        .current_dir(workspace_root())
        .output()
        .unwrap_or_else(|e| die(&format!("spawn boot bench-once: {e}")));

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
            job.label(),
            stderr.lines().rev().take(8).collect::<Vec<_>>().join("\n")
        ));
    }

    let witnesses = parse_witness_lines(&stderr).unwrap_or_else(|errs| {
        let lines: Vec<String> = errs.iter().map(ToString::to_string).collect();
        die(&format!(
            "{}: malformed witness lines:\n  {}",
            job.label(),
            lines.join("\n  ")
        ))
    });

    let mut results = stdout.lines().filter(|l| l.starts_with("BENCH_RESULT"));
    let result = results
        .next()
        .unwrap_or_else(|| die(&format!("{}: no BENCH_RESULT line", job.label())));
    // One boot prints the line once. Two lines mean two runs' output
    // reached one pipe, and neither can be attributed -- the same
    // reason a repeated RUN_IDENTITY line is refused.
    if results.next().is_some() {
        die(&format!(
            "{}: more than one BENCH_RESULT line; refusing to record from output that \
             cannot be attributed to one run",
            job.label()
        ));
    }
    let mut steps = None;
    let mut budget = None;
    let mut outcome = None;
    for tok in result.split_whitespace() {
        if let Some(v) = tok.strip_prefix("steps=") {
            steps = Some(parse_result_field(job, "steps", v));
        } else if let Some(v) = tok.strip_prefix("budget=") {
            budget = Some(parse_result_field(job, "budget", v));
        } else if let Some(v) = tok.strip_prefix("outcome=") {
            outcome = Some(v.to_string());
        }
    }
    let steps =
        steps.unwrap_or_else(|| die(&format!("{}: BENCH_RESULT has no steps=", job.label())));
    let budget =
        budget.unwrap_or_else(|| die(&format!("{}: BENCH_RESULT has no budget=", job.label())));
    let outcome =
        outcome.unwrap_or_else(|| die(&format!("{}: BENCH_RESULT has no outcome=", job.label())));
    // The boot prints the line even when the store names nothing, with
    // an empty payload. A missing line therefore means the child was
    // not this binary, or its stderr never arrived.
    let identity = RunIdentity::parse_sentinel_lines(&stderr)
        .unwrap_or_else(|e| die(&format!("{}: {e}", job.label())))
        .unwrap_or_else(|| {
            die(&format!(
                "{}: the boot printed no {RUN_IDENTITY_SENTINEL} line; refusing to record an \
                 anchor that cannot name what it was measured against",
                job.label()
            ))
        });
    let disagreements = cell_disagreements(&identity, &job.cell);
    if !disagreements.is_empty() {
        die(&format!(
            "{}: the run {}; refusing to file a measurement under a cell it did not \
             compose. The anchor tree is keyed by the composed triple, so the gate would \
             hold one configuration against another configuration's run",
            job.label(),
            disagreements.join(", and ")
        ));
    }
    Some(Measurement {
        witnesses: witnesses.values,
        steps,
        budget: Budget::new(budget),
        outcome,
        identity,
    })
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

/// The cell's committed anchor, or `None` when it has never been
/// recorded.
///
/// Only an absent file means "never recorded". An unreadable or
/// unparseable one names itself.
fn read_previous_anchor(job: &Job, path: &Path) -> Option<BootSummary> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => die(&format!(
            "{}: read {}: {e}; refusing to treat an unreadable anchor as absent",
            job.label(),
            path.display()
        )),
    };
    Some(serde_json::from_str(&text).unwrap_or_else(|e| {
        die(&format!(
            "{}: parse {}: {e}; the anchor exists but is malformed. Repair it rather than \
             letting this run recreate it without its recorded witness classes.",
            job.label(),
            path.display()
        ))
    }))
}

/// Rewrite one cell's anchor and keep any hand-promoted witness class.
///
/// Returns `false` when the title is not installed: `--all` skips it by
/// name, a named scope treats it as an error.
fn record_one(job: &Job, strict: bool) -> bool {
    let Some(Measurement {
        witnesses,
        steps,
        budget,
        outcome,
        identity,
    }) = measure(job)
    else {
        if strict {
            die(&format!(
                "{}: the title's dump is not installed",
                job.label()
            ));
        }
        println!("{}: skipped -- not installed on this machine", job.label());
        return false;
    };

    let root = workspace_root();
    let path = boot_anchor_path(&root, &job.content_id, &job.cell);
    let previous = read_previous_anchor(job, &path);
    let previous_identity = previous
        .as_ref()
        .map(|p| p.identity.clone())
        .unwrap_or_default();

    // History is parsed BEFORE the baseline is written: a malformed
    // history line must abort while the anchor is still untouched,
    // never leave a moved anchor with no history entry.
    let hist_path = history_path(&root, &job.content_id, &job.cell);
    let existing_history = read_history(&hist_path);
    let history_entries = boot_history::parse(&existing_history)
        .unwrap_or_else(|e| die(&format!("parse {}: {e}", hist_path.display())));
    let history_entry = BootHistoryEntry::new_if_changed(
        history_entries.last(),
        steps,
        &outcome,
        witnesses.clone(),
        identity.clone(),
    );

    let outcome_parsed = BootOutcome::from_str(&outcome).unwrap_or_else(|e| {
        die(&format!(
            "{}: outcome {outcome:?} did not parse: {e}",
            job.label()
        ))
    });
    let mut summary = BootSummary::new_with_breaks(
        checkpoint_kind(job.checkpoint),
        outcome_parsed,
        steps,
        budget,
        witnesses.get("host_invariant_breaks").copied().unwrap_or(0),
    )
    .unwrap_or_else(|e| {
        die(&format!(
            "{}: recorded summary is invalid: {e}",
            job.label()
        ))
    });
    summary.witnesses = record(previous.as_ref().map(|p| &p.witnesses), &witnesses);
    summary.identity = identity;

    let dir = path
        .parent()
        .unwrap_or_else(|| die(&format!("{} has no parent directory", path.display())));
    std::fs::create_dir_all(dir).unwrap_or_else(|e| die(&format!("create {}: {e}", dir.display())));
    let json = serde_json::to_string_pretty(&summary)
        .unwrap_or_else(|e| die(&format!("serialize {}: {e}", path.display())));
    std::fs::write(&path, json + "\n")
        .unwrap_or_else(|e| die(&format!("write {}: {e}", path.display())));

    // The history's move rule compares against the previous history
    // line, which can carry no triple. The anchor's own previous triple
    // is the only record that can say the measurement basis moved.
    if !previous_identity.is_empty() && previous_identity != summary.identity {
        println!(
            "{}: measured against a different triple than the previous anchor named",
            job.label()
        );
        for line in previous_identity.render_lines() {
            println!("  was  {line}");
        }
        for line in summary.identity.render_lines() {
            println!("  now  {line}");
        }
    }

    match history_entry {
        None => println!(
            "{}: unchanged ({outcome}, {steps} steps, {} witnesses)",
            job.label(),
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
                job.label(),
                entry_line.changed.join(", ")
            );
        }
    }
    true
}

/// Refuse a `--registry` the spawned measurement cannot honour.
///
/// `measure` re-enters the binary as `boot bench-once --title <name>`,
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
             The boot is re-entered as `boot bench-once --title <name>`, which resolves \
             names against {}, so the anchor would be measured from one manifest and \
             filed under another. Point --registry at {} or drop it.",
            registry.display(),
            default.display(),
            default.display(),
        ));
    }
}

/// The one title `--title` names, or every registered title.
fn select_titles<'a>(titles: &'a [TitleManifest], one: Option<&str>) -> Vec<&'a TitleManifest> {
    let Some(name) = one else {
        return titles.iter().collect();
    };
    let Some(hit) = titles.iter().find(|t| t.short_name == name) else {
        let known: Vec<&str> = titles.iter().map(|t| t.short_name.as_str()).collect();
        die(&format!(
            "unknown title {name:?}; registry has: {}",
            known.join(", ")
        ));
    };
    vec![hit]
}

/// Narrow one title's declared cells to those `--fw` / `--game-ver`
/// name, and refuse a cell the manifest does not declare.
fn filter_declared(jobs: Vec<Job>, fw: Option<&str>, game_ver: Option<&str>) -> Vec<Job> {
    if fw.is_none() && game_ver.is_none() {
        return jobs;
    }
    let declared: Vec<String> = jobs.iter().map(|j| j.cell.label()).collect();
    let kept: Vec<Job> = jobs
        .into_iter()
        .filter(|j| {
            fw.is_none_or(|f| j.cell.fw == f)
                && game_ver.is_none_or(|v| j.cell.game_ver.as_deref() == Some(v))
        })
        .collect();
    if kept.is_empty() {
        let asked = match (fw, game_ver) {
            (Some(f), Some(v)) => format!("fw {f} x {v}"),
            (Some(f), None) => format!("fw {f}"),
            (None, Some(v)) => format!("game version {v}"),
            (None, None) => unreachable!("an unfiltered selection returned above"),
        };
        die(&format!(
            "record-anchors: the registry declares no cell matching {asked}; declared: {}. \
             The gate reads declared cells, so an anchor recorded outside the declaration \
             would be compared against by nothing. Add the row to [[bench.matrix]] first",
            if declared.is_empty() {
                "none".to_string()
            } else {
                declared.join(", ")
            }
        ));
    }
    kept
}

pub(crate) fn run(args: &RecordAnchorsArgs, render: RenderFlags) {
    let default_registry = workspace_root().join(DEFAULT_TITLE_REGISTRY_DIR);
    let registry = match &args.registry {
        Some(given) => {
            reject_unforwardable_registry(given, &default_registry);
            given.clone()
        }
        None => default_registry,
    };

    let titles = read_registry(&registry);
    if titles.is_empty() {
        die(&format!("no title manifests under {}", registry.display()));
    }

    let one = args.scope.title.as_deref();
    let selected = select_titles(&titles, one);
    let undeclared: Vec<&str> = selected
        .iter()
        .filter(|t| t.matrix.is_empty())
        .map(|t| t.short_name.as_str())
        .collect();
    // A title with a PARAM.SFO always declares the cell its `system_ver`
    // derives. Only a title shipped inside the firmware, or built beside
    // its manifest, can reach here with nothing declared.
    if !undeclared.is_empty() {
        die(&format!(
            "no cells declared for: {}. An anchor is keyed by (content id, firmware, game \
             version), and a title with no floor of its own declares its cells as \
             [[bench.matrix]] rows alone; with none it has nothing to record and nothing \
             for the gate to read",
            undeclared.join(", ")
        ));
    }
    let jobs = filter_declared(
        selected.into_iter().flat_map(jobs_for).collect(),
        args.fw.as_deref(),
        args.game_ver.as_deref(),
    );
    let narrowed = args.fw.is_some() || args.game_ver.is_some();
    let (jobs, pending) = skip_pending(jobs, narrowed);
    for job in &pending {
        println!(
            "{}: skipped -- declared pending ({})",
            job.label(),
            job.pending.as_deref().unwrap_or_default()
        );
    }
    if jobs.is_empty() {
        die(&format!(
            "every selected cell is declared pending ({}); nothing to record. Name one with \
             --fw / --game-ver to measure it regardless",
            pending
                .iter()
                .map(Job::label)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    let strict = one.is_some();
    let mut recorded = 0usize;
    let total = jobs.len();
    let bar = ProgressBar::start(render.caps(), &RECORD_ANCHORS_TASK, "cells");
    let sink = bar.sink();
    sink.totals(0, total as u64);
    for (index, job) in jobs.iter().enumerate() {
        sink.item_started(&format!("{} ({}/{total})", job.label(), index + 1));
        if record_one(job, strict) {
            recorded += 1;
        }
        sink.advanced(1);
    }
    sink.finished();
    bar.finish();
    // --all over a machine with zero installed titles must not exit 0
    // having recorded nothing.
    if recorded == 0 {
        die(&format!(
            "none of the {total} declared cell(s) is installed; nothing recorded"
        ));
    }
}

#[cfg(test)]
#[path = "tests/record_anchors_tests.rs"]
mod tests;
