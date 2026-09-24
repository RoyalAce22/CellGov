//! The anchor: the committed `BootSummary` of one cell, and the one
//! comparison that holds a measured run against it.
//!
//! `boot bench` and the title-witness suite read an anchor through
//! [`load_anchor`] and compare through [`hold_against_anchor`], so a
//! run one of them passes cannot fail the other. `dev record-anchors`
//! reads the previous anchor through the same loader.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use cellgov_time::Budget;

use crate::boot_summary::{BootSummary, BootSummaryError, CheckpointKind};
use crate::identity::RUN_IDENTITY_SENTINEL;
use crate::identity::{BootOverrides, FirmwareIdentity, GameIdentity, RunIdentity};
use crate::runner_cellgov::BootOutcome;
use crate::witness_parse::{parse_witness_lines, ParsedWitnesses, UnsupportedSyscallWitness};
use crate::witnesses::{check_all, record, unrecorded};

/// How a run compared against its cell's committed anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorVerdict {
    /// The check was not requested.
    Skipped,
    /// The comparison is meaningless for this invocation; each string
    /// names one cause.
    NotComparable(Vec<String>),
    /// No anchor is committed for the cell this run composed, so there
    /// is nothing to compare against. The string names the cell.
    NotRecorded(String),
    /// The run reproduced every recorded value.
    Match,
    /// Every disagreement found, in the order the comparison made them.
    Drift(Vec<String>),
}

/// Why a committed anchor did not load.
#[derive(Debug, thiserror::Error)]
pub enum AnchorLoadError {
    /// The file exists, but reading it failed.
    #[error("read {}: {source}", path.display())]
    Read {
        /// The anchor file.
        path: PathBuf,
        /// The read failure.
        #[source]
        source: io::Error,
    },
    /// The file holds no valid `BootSummary`.
    #[error("parse {}: {source}", path.display())]
    Parse {
        /// The anchor file.
        path: PathBuf,
        /// The decode failure.
        #[source]
        source: serde_json::Error,
    },
}

/// The anchor at `path`, or `None` when nobody has recorded it.
///
/// Only an absent file means "never recorded". An unreadable or
/// unparseable one is an error, so a damaged anchor cannot pass for a
/// cell nobody measured.
pub fn load_anchor(path: &Path) -> Result<Option<BootSummary>, AnchorLoadError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(AnchorLoadError::Read {
                path: path.to_path_buf(),
                source,
            })
        }
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|source| AnchorLoadError::Parse {
            path: path.to_path_buf(),
            source,
        })
}

/// What one measured run produced, in the terms the anchor records.
#[derive(Debug, Clone, Copy)]
pub struct MeasuredRun<'a> {
    /// Stop condition the run was taken at.
    pub checkpoint: CheckpointKind,
    /// Steps the run retired.
    pub steps: u64,
    /// Instructions each step was granted.
    pub budget: Budget,
    /// How the run ended.
    pub outcome: BootOutcome,
    /// The measuring process's own stderr: the witness lines, and the
    /// `RUN_IDENTITY` line naming what it composed.
    pub stderr: &'a str,
}

/// Hold `run` against `baseline` and return every disagreement.
///
/// The identity triple comes from the measuring child's own stream,
/// the same stream as the steps and the witnesses. An identity the
/// parent resolved would hide the mismatch this comparison exists to
/// catch. A stream whose witness lines do not parse, or that names no
/// identity, is a disagreement.
pub fn hold_against_anchor(baseline: &BootSummary, run: &MeasuredRun<'_>) -> Vec<String> {
    let observed = match parse_witness_lines(run.stderr) {
        Ok(w) => w,
        Err(errs) => {
            return errs
                .iter()
                .map(|e| format!("malformed witness line: {e}"))
                .collect();
        }
    };
    let identity = match RunIdentity::parse_sentinel_lines(run.stderr) {
        Ok(Some(i)) => i,
        Ok(None) => {
            return vec![format!(
                "the measured run printed no {RUN_IDENTITY_SENTINEL} line, so nothing says \
                 which firmware and title version produced these numbers"
            )]
        }
        Err(e) => return vec![e.to_string()],
    };
    anchor_disagreements(
        baseline,
        &identity,
        run.checkpoint,
        run.steps,
        run.budget,
        run.outcome,
        &observed,
    )
}

/// Compare a run against a loaded anchor, returning every
/// disagreement.
fn anchor_disagreements(
    baseline: &BootSummary,
    identity: &RunIdentity,
    checkpoint: CheckpointKind,
    steps: u64,
    budget: Budget,
    outcome: BootOutcome,
    observed: &ParsedWitnesses,
) -> Vec<String> {
    let mut failures = mislabelled_anchor(&baseline.identity, identity);
    // The checkpoint is a manifest row an edit can move after the
    // anchor was recorded. A stop condition the run never reaches
    // leaves the steps, the outcome and the witnesses intact, so
    // nothing else in this comparison sees the move.
    if checkpoint != baseline.checkpoint {
        failures.push(format!(
            "checkpoint {} != recorded {}",
            checkpoint.as_markdown_label(),
            baseline.checkpoint.as_markdown_label()
        ));
    }
    if steps != baseline.steps {
        failures.push(format!("steps {steps} != recorded {}", baseline.steps));
    }
    // `steps * budget` is the anchor's instruction count, so a moved
    // budget retires a different trajectory under an unmoved step count.
    if budget != baseline.budget {
        failures.push(format!("budget {budget} != recorded {}", baseline.budget));
    }
    if outcome != baseline.outcome {
        failures.push(format!(
            "outcome {outcome} != recorded {}",
            baseline.outcome
        ));
    }
    if baseline.witnesses.is_empty() {
        failures.push("anchor records no witnesses".to_string());
        return failures;
    }
    for failure in check_all(&baseline.witnesses, observed) {
        failures.push(failure.to_string());
    }
    for name in unrecorded(&baseline.witnesses, &observed.values) {
        failures.push(format!(
            "witness {name} is emitted but not recorded in the anchor"
        ));
    }
    if baseline.unsupported_syscalls != observed.unsupported_syscalls {
        failures.push("unsupported syscall inventory differs from recorded anchor".to_string());
    }
    failures
}

/// How the identity a summary embeds disagrees with the one the run
/// composed.
///
/// The anchor's directory names the cell it is filed under. The
/// embedded identity is what the recording run itself composed. A
/// disagreement means the file measured one of these:
///
/// - another cell, when the firmware or game half differs;
/// - another trajectory of the same cell, when the override set differs.
fn mislabelled_anchor(recorded: &RunIdentity, run: &RunIdentity) -> Vec<String> {
    let mut failures = Vec::new();
    if recorded.firmware != run.firmware {
        failures.push(format!(
            "the anchor was measured against a different firmware: recorded {}, ran {}",
            render_firmware(recorded.firmware.as_ref()),
            render_firmware(run.firmware.as_ref()),
        ));
    }
    if recorded.game != run.game {
        failures.push(format!(
            "the anchor was measured against a different title version: recorded {}, ran {}",
            render_game(recorded.game.as_ref()),
            render_game(run.game.as_ref()),
        ));
    }
    if recorded.overrides != run.overrides {
        failures.push(format!(
            "the anchor was measured under different boot overrides: recorded {}, ran {}",
            render_overrides(&recorded.overrides),
            render_overrides(&run.overrides),
        ));
    }
    failures
}

/// How a report names an override set.
fn render_overrides(overrides: &BootOverrides) -> String {
    if overrides.is_empty() {
        "(none)".to_string()
    } else {
        overrides.names().join(" ")
    }
}

/// How a report names the firmware half.
///
/// The comparison covers every field, so the report renders every
/// field. Two entries installed from different PUPs can carry one
/// version string. The version alone would then print the same value
/// on both sides of a disagreement.
fn render_firmware(half: Option<&FirmwareIdentity>) -> String {
    half.map_or_else(unidentified, |f| {
        format!(
            "{} (image {}, pup sha256 {})",
            f.version, f.image_version, f.pup_sha256
        )
    })
}

/// How a report names the game half. Renders every compared field for
/// the reason [`render_firmware`] gives.
fn render_game(half: Option<&GameIdentity>) -> String {
    half.map_or_else(unidentified, |g| {
        format!("{} {} ({})", g.title_id, g.version, g.app_version_label())
    })
}

/// The half a run had nothing to name.
fn unidentified() -> String {
    "(unidentified)".to_string()
}

/// One measurement, as `dev record-anchors` files it.
#[derive(Debug, Clone)]
pub struct AnchorMeasurement {
    /// Stop condition the run was taken at.
    pub checkpoint: CheckpointKind,
    /// How the run ended.
    pub outcome: BootOutcome,
    /// Steps the run retired.
    pub steps: u64,
    /// Instructions each step was granted.
    pub budget: Budget,
    /// Every witness value the run emitted.
    pub witnesses: BTreeMap<String, u64>,
    /// The unsupported-syscall inventory the run emitted.
    pub unsupported_syscalls: BTreeMap<u64, UnsupportedSyscallWitness>,
    /// What the run composed.
    pub identity: RunIdentity,
}

/// The anchor `measurement` records.
///
/// A witness keeps the class `previous` promoted it to, so re-recording
/// an anchor never loosens an exact witness back to an at-least bound.
pub fn anchor_from_measurement(
    previous: Option<&BootSummary>,
    measurement: AnchorMeasurement,
) -> Result<BootSummary, BootSummaryError> {
    let mut summary = BootSummary::new_with_breaks(
        measurement.checkpoint,
        measurement.outcome,
        measurement.steps,
        measurement.budget,
        measurement
            .witnesses
            .get("host_invariant_breaks")
            .copied()
            .unwrap_or(0),
    )?;
    summary.witnesses = record(previous.map(|p| &p.witnesses), &measurement.witnesses);
    summary.unsupported_syscalls = measurement.unsupported_syscalls;
    summary.identity = measurement.identity;
    Ok(summary)
}

#[cfg(test)]
#[path = "tests/anchor_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/anchor_hold_tests.rs"]
mod hold_tests;

#[cfg(test)]
#[path = "tests/anchor_override_tests.rs"]
mod override_tests;
