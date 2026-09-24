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

use cellgov_compare::bench::{
    anchor_from_measurement, load_anchor, parse_bench_result, AnchorLoadError, AnchorMeasurement,
    ParseBenchError,
};
use cellgov_compare::boot_history::{self, BootHistoryEntry};
use cellgov_compare::runner_cellgov::BootOutcome;
use cellgov_compare::witness_parse::parse_witness_lines;
use cellgov_compare::witness_parse::UnsupportedSyscallWitness;
use cellgov_compare::witnesses::{BOOT_STARTED_SENTINEL, TITLE_NOT_INSTALLED_SENTINEL};
use cellgov_compare::{BootSummary, RunIdentity, RUN_IDENTITY_SENTINEL};
use cellgov_terminal::caps::RenderFlags;
use cellgov_terminal::progress::{ProgressBar, ProgressSink as _};
use cellgov_time::Budget;

use crate::cli::declared_cells::{
    declared_cells, filter_declared, read_registry, refuse_undeclared, select_titles,
    split_pending, DeclaredCell,
};
use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::title::DEFAULT_TITLE_REGISTRY_DIR;
use cellgov_boot::manifest::{CellDisagreement, CellKey};

use crate::paths::{boot_anchor_path, history_path, workspace_root};
use crate::progress::RECORD_ANCHORS_TASK;

use crate::cli::parse::RecordAnchorsArgs;

/// One declared cell to re-measure.
type Job = DeclaredCell;

struct Measurement {
    witnesses: BTreeMap<String, u64>,
    unsupported_syscalls: BTreeMap<u64, UnsupportedSyscallWitness>,
    steps: u64,
    budget: Budget,
    outcome: BootOutcome,
    identity: RunIdentity,
}

/// Every way the run's own identity contradicts the cell that would
/// receive its result.
///
/// An applied boot override contradicts every cell, because no anchor
/// records one.
///
/// The child re-resolves its composition, and not every flag reaches
/// that resolution. A boot asked for no firmware never consults `--fw`;
/// it composes a firmware-free run whatever the flag named
/// (`resolve_composition` in `boot_cmd`). The identity line reports what
/// the run composed, so it decides which cell may receive the result.
fn cell_disagreements(identity: &RunIdentity, cell: &CellKey) -> Vec<String> {
    // A fresh run states what it composed, so an identity naming no
    // firmware or no game entry disagrees with a cell that names one.
    cell.disagreements(identity)
        .into_iter()
        .map(|d| match d {
            // `measure` passes no override flag, so this names a child
            // that applied one anyway.
            CellDisagreement::Overridden { names } => format!(
                "applied boot override(s) {}, which no anchor is recorded under",
                names.join(" ")
            ),
            CellDisagreement::FirmwareMismatch { cell, recorded } => {
                format!("composed firmware {recorded} rather than {cell}")
            }
            CellDisagreement::NoFirmware { cell } => {
                format!("composed no managed firmware rather than firmware {cell}")
            }
            CellDisagreement::GameVersionMismatch { cell, recorded } => format!(
                "composed game version {recorded} rather than {}",
                cell.as_deref().unwrap_or("(none)")
            ),
            CellDisagreement::NoGameVersion { cell } => {
                format!("composed game version (none) rather than {cell}")
            }
        })
        .collect()
}

/// Boots one cell, or returns `None` when its dump is not installed.
///
/// # Errors
///
/// Returns an error for every failure except the title-not-installed marker.
fn measure(job: &Job) -> Result<Option<Measurement>, CommandError> {
    let exe = std::env::current_exe()
        .map_err(|error| CommandError::failed(format!("current_exe: {error}")))?;
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
        .map_err(|error| CommandError::failed(format!("spawn boot bench-once: {error}")))?;
    super::exit::propagate_interrupt(output.status)?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        if stderr.contains(TITLE_NOT_INSTALLED_SENTINEL) {
            return Ok(None);
        }
        let phase = if stderr.contains(BOOT_STARTED_SENTINEL) {
            "boot started then failed"
        } else {
            "boot inputs failed to resolve (dump present but unusable?)"
        };
        return Err(CommandError::failed(format!(
            "{}: {phase}; refusing to record a broken run. stderr tail:\n{}",
            job.label(),
            stderr.lines().rev().take(8).collect::<Vec<_>>().join("\n")
        )));
    }

    let witnesses = parse_witness_lines(&stderr).map_err(|errors| {
        let lines: Vec<String> = errors.iter().map(ToString::to_string).collect();
        CommandError::failed(format!(
            "{}: malformed witness lines:\n  {}",
            job.label(),
            lines.join("\n  ")
        ))
    })?;

    let parsed = parse_bench_result(&stdout).map_err(|error| match error {
        // One boot prints the line once. Two lines mean two runs'
        // output reached one pipe, and neither can be attributed -- the
        // same reason a repeated RUN_IDENTITY line is refused.
        ParseBenchError::DuplicateResultLine => CommandError::failed(format!(
            "{}: more than one BENCH_RESULT line; refusing to record from output that \
             cannot be attributed to one run",
            job.label()
        )),
        other => CommandError::failed(format!("{}: {other}", job.label())),
    })?;
    for warning in &parsed.warnings {
        eprintln!("{}: warning: {warning}", job.label());
    }
    let result = parsed.result;
    // The boot prints the line even when the store names nothing, with
    // an empty payload. A missing line therefore means the child was
    // not this binary, or its stderr never arrived.
    let identity = RunIdentity::parse_sentinel_lines(&stderr)
        .map_err(|error| CommandError::failed(format!("{}: {error}", job.label())))?
        .ok_or_else(|| {
            CommandError::failed(format!(
                "{}: the boot printed no {RUN_IDENTITY_SENTINEL} line; refusing to record an \
                 anchor that cannot name what it was measured against",
                job.label()
            ))
        })?;
    let disagreements = cell_disagreements(&identity, &job.cell);
    if !disagreements.is_empty() {
        return Err(CommandError::failed(format!(
            "{}: the run {}; refusing to file the measurement under that cell. The \
             anchor tree is keyed by the composed identity triple and records no boot \
             override, so the gate would hold one configuration against another \
             configuration's run",
            job.label(),
            disagreements.join(", and ")
        )));
    }
    Ok(Some(Measurement {
        witnesses: witnesses.values,
        unsupported_syscalls: witnesses.unsupported_syscalls,
        steps: result.steps as u64,
        budget: result.budget,
        outcome: result.outcome,
        identity,
    }))
}

/// Reads the existing append-only history.
///
/// A missing file is an empty history. The command returns every other read failure and leaves the history unchanged.
fn read_history(path: &Path) -> Result<String, CommandError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(CommandError::failed(format!(
            "read {}: {e}; refusing to treat an unreadable history as empty",
            path.display(),
            e = error,
        ))),
    }
}

/// The cell's committed anchor, or `None` when it has never been
/// recorded.
///
/// Only an absent file means "never recorded". An unreadable or
/// unparseable one names itself.
fn read_previous_anchor(job: &Job, path: &Path) -> Result<Option<BootSummary>, CommandError> {
    load_anchor(path).map_err(|error| match error {
        AnchorLoadError::Read { .. } => CommandError::failed(format!(
            "{}: {error}; refusing to treat an unreadable anchor as absent",
            job.label(),
        )),
        AnchorLoadError::Parse { .. } => CommandError::failed(format!(
            "{}: {error}; the anchor exists but is malformed. Repair it rather than \
             letting this run recreate it without its recorded witness classes.",
            job.label(),
        )),
    })
}

/// Rewrites one cell's anchor and preserves its promoted witness classes.
///
/// With `--all`, an uninstalled title returns `Ok(false)`. A named title returns an error.
fn record_one(job: &Job, strict: bool) -> Result<bool, CommandError> {
    let Some(Measurement {
        witnesses,
        unsupported_syscalls,
        steps,
        budget,
        outcome: measured_outcome,
        identity,
    }) = measure(job)?
    else {
        if strict {
            return Err(CommandError::failed(format!(
                "{}: the title's dump is not installed",
                job.label()
            )));
        }
        println!("{}: skipped -- not installed on this machine", job.label());
        return Ok(false);
    };

    let root = workspace_root();
    let path = boot_anchor_path(&root, &job.content_id, &job.cell);
    let previous = read_previous_anchor(job, &path)?;
    let previous_identity = previous
        .as_ref()
        .map(|p| p.identity.clone())
        .unwrap_or_default();

    // History is parsed before the baseline is written: a malformed
    // history line must abort while the anchor is still untouched,
    // never leave a moved anchor with no history entry.
    let hist_path = history_path(&root, &job.content_id, &job.cell);
    let existing_history = read_history(&hist_path)?;
    let history_entries = boot_history::parse(&existing_history)
        .map_err(|error| CommandError::failed(format!("parse {}: {error}", hist_path.display())))?;
    // The history spells an outcome the way `BootOutcome`'s `FromStr`
    // reads it back: its Display form.
    let outcome = measured_outcome.to_string();
    let history_entry = BootHistoryEntry::new_if_changed(
        history_entries.last(),
        steps,
        &outcome,
        witnesses.clone(),
        identity.clone(),
    );

    let summary = anchor_from_measurement(
        previous.as_ref(),
        AnchorMeasurement {
            checkpoint: job.checkpoint.kind(),
            outcome: measured_outcome,
            steps,
            budget,
            witnesses,
            unsupported_syscalls,
            identity,
        },
    )
    .map_err(|error| {
        CommandError::failed(format!(
            "{}: recorded summary is invalid: {e}",
            job.label(),
            e = error,
        ))
    })?;

    let dir = path.parent().ok_or_else(|| {
        CommandError::failed(format!("{} has no parent directory", path.display()))
    })?;
    std::fs::create_dir_all(dir)
        .map_err(|error| CommandError::failed(format!("create {}: {error}", dir.display())))?;
    let json = serde_json::to_string_pretty(&summary)
        .map_err(|error| CommandError::failed(format!("serialize {}: {error}", path.display())))?;
    std::fs::write(&path, json + "\n")
        .map_err(|error| CommandError::failed(format!("write {}: {error}", path.display())))?;

    // The history's move rule compares against the previous history
    // line, which can carry no identity triple. The anchor's own
    // previous identity triple is the only record that can say the
    // measurement basis moved.
    if !previous_identity.is_empty() && previous_identity != summary.identity {
        println!(
            "{}: measured against a different identity triple than the previous anchor named",
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
            let line = boot_history::render_line(&entry_line).map_err(|error| {
                CommandError::failed(format!("serialize history entry: {error}"))
            })?;
            let mut text = existing_history;
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&line);
            std::fs::write(&hist_path, text).map_err(|error| {
                CommandError::failed(format!("write {}: {error}", hist_path.display()))
            })?;
            println!(
                "{}: moved ({outcome}, {steps} steps) -- {}",
                job.label(),
                entry_line.changed.join(", ")
            );
        }
    }
    Ok(true)
}

/// Refuse a `--registry` the spawned measurement cannot honour.
///
/// `measure` re-enters the binary as `boot bench-once --title <name>`,
/// and that path resolves the name against the compiled-in registry
/// directory. A `--registry` pointing elsewhere would enumerate one
/// set of manifests, boot the same-named title from another, and then
/// write the anchor under the first manifest's content id.
fn reject_unforwardable_registry(registry: &Path, default: &Path) -> Result<(), CommandError> {
    let same = match (registry.canonicalize(), default.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => registry == default,
    };
    if !same {
        return Err(CommandError::failed(format!(
            "record-anchors: --registry {} is not the registry the measurement reads. \
             The boot is re-entered as `boot bench-once --title <name>`, which resolves \
             names against {}, so the anchor would be measured from one manifest and \
             filed under another. Point --registry at {} or drop it.",
            registry.display(),
            default.display(),
            default.display(),
        )));
    }
    Ok(())
}

pub(crate) fn run(
    args: &RecordAnchorsArgs,
    render: RenderFlags,
) -> Result<CommandExitCode, CommandError> {
    let default_registry = workspace_root().join(DEFAULT_TITLE_REGISTRY_DIR);
    let registry = match &args.registry {
        Some(given) => {
            reject_unforwardable_registry(given, &default_registry)?;
            given.clone()
        }
        None => default_registry,
    };

    let titles = read_registry(&registry)?;
    if titles.is_empty() {
        return Err(CommandError::failed(format!(
            "no title manifests under {}",
            registry.display()
        )));
    }

    let one = args.scope.title.as_deref();
    let selected = select_titles(&titles, one)?;
    refuse_undeclared(&selected)?;
    let jobs = filter_declared(
        selected.into_iter().flat_map(declared_cells).collect(),
        args.fw.as_deref(),
        args.game_ver.as_deref(),
        "record-anchors",
    )?;
    // A named selection asks for the measurement of a pending cell
    // regardless; once the anchor exists, the structure gate says to
    // drop the marker.
    let narrowed = args.fw.is_some() || args.game_ver.is_some();
    let (jobs, pending) = split_pending(jobs, narrowed);
    for job in &pending {
        println!(
            "{}: skipped -- declared pending ({})",
            job.label(),
            job.pending.as_deref().unwrap_or_default()
        );
    }
    if jobs.is_empty() {
        return Err(CommandError::failed(format!(
            "every selected cell is declared pending ({}); nothing to record. Name one with \
             --fw / --game-ver to measure it regardless",
            pending
                .iter()
                .map(Job::label)
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }

    let strict = one.is_some();
    let mut recorded = 0usize;
    let total = jobs.len();
    let bar = ProgressBar::start(render.caps(), &RECORD_ANCHORS_TASK, "cells");
    let sink = bar.sink();
    sink.totals(0, total as u64);
    for (index, job) in jobs.iter().enumerate() {
        sink.item_started(&format!("{} ({}/{total})", job.label(), index + 1));
        if record_one(job, strict)? {
            recorded += 1;
        }
        sink.advanced(1);
    }
    sink.finished();
    bar.finish();
    // --all over a machine with zero installed titles must not exit 0
    // having recorded nothing.
    if recorded == 0 {
        return Err(CommandError::failed(format!(
            "none of the {total} declared cell(s) is installed; nothing recorded"
        )));
    }
    Ok(CommandExitCode::SUCCESS)
}

#[cfg(test)]
#[path = "tests/record_anchors_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/record_anchors_override_tests.rs"]
mod override_tests;
