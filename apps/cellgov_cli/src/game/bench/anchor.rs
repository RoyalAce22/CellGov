//! The check that holds one measured run against the committed anchor
//! for its cell. It also names every reason a run is incomparable.

use std::path::Path;

use cellgov_compare::witness_parse::{parse_witness_lines, ParsedWitnesses};
use cellgov_compare::witnesses::{check_all, unrecorded};
use cellgov_compare::{
    BootOverrides, BootSummary, FirmwareIdentity, GameIdentity, RunIdentity, RUN_IDENTITY_SENTINEL,
};
use cellgov_time::Budget;

use super::options::BenchOptions;
use crate::paths::{boot_anchor_path, workspace_root};
use cellgov_boot::manifest::{self, CellKey};

/// How a run compared against its cell's committed anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorVerdict {
    /// The check was not requested.
    Skipped,
    /// The comparison is meaningless for this invocation; each string
    /// names one cause.
    NotComparable(Vec<String>),
    /// No anchor is committed for the cell this run composed, so there
    /// is nothing to compare against.
    NotRecorded(String),
    /// The run reproduced every recorded value.
    Match,
    /// Every disagreement found, in the order the comparison made them.
    Drift(Vec<String>),
}

/// Why this invocation cannot be held against the cell's committed
/// anchor.
///
/// The anchor is measured by `dev record-anchors`, which boots the cell
/// under what its manifest row declares and nothing else. An override
/// that moves the trajectory yields a legitimately different run, so
/// gating it would report a regression that is not one. `--prescan` is
/// absent from the list because it only prints a decode report before
/// execution.
pub(super) fn incomparable_reasons(opts: &BenchOptions<'_>) -> Vec<String> {
    let mut reasons = Vec::new();
    if let Some(dir) = opts.selection.firmware_dir {
        reasons.push(format!(
            "--firmware-dir {dir} is unmanaged: the run carries no firmware version, so nothing \
             names the cell an anchor would be filed under"
        ));
    } else if opts.plan.cell.is_none() {
        reasons.push(format!(
            "{} composed no cell: an anchor is keyed by (content id, firmware, game version), \
             and this run named no firmware version or no game version to key on",
            opts.title.name()
        ));
    }
    if opts.max_steps as u64 != opts.plan.max_steps {
        reasons.push(format!(
            "--max-steps {} differs from the {} the cell's anchor is recorded under",
            opts.max_steps, opts.plan.max_steps
        ));
    }
    if let Some(cp) = opts.checkpoint_override {
        if cp != opts.plan.checkpoint {
            reasons.push(format!(
                "--checkpoint {} overrides the cell's checkpoint {}",
                cp.as_cli_str(),
                opts.plan.checkpoint.as_cli_str()
            ));
        }
    }
    if let Some(b) = opts.budget_override {
        reasons.push(format!("--budget {b} overrides the manifest budget"));
    }
    if opts.strict_reserved {
        reasons.push("--strict-reserved changes reserved-region write handling".to_string());
    }
    if !opts.guest_args.is_empty() {
        reasons.push(format!(
            "--guest-arg supplies {} guest argv entries; the anchor is recorded with none",
            opts.guest_args.len()
        ));
    }
    for (flag, value) in crate::cli::parse::override_flags(&opts.identity.overrides) {
        let spelled = value.map_or_else(|| flag.to_string(), |v| format!("{flag} {v}"));
        reasons.push(format!(
            "{spelled} overrides boot behaviour; the anchor is recorded with no boot override"
        ));
    }
    reasons
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

/// Compare a run against a loaded anchor, returning every
/// disagreement.
///
/// Mirrors the comparison in `tests/title_witnesses.rs`: the two must
/// agree, or `boot bench` would pass a run the witness suite rejects.
fn anchor_disagreements(
    baseline: &BootSummary,
    identity: &RunIdentity,
    checkpoint: manifest::CheckpointTrigger,
    steps: u64,
    budget: Budget,
    outcome: &str,
    observed: &ParsedWitnesses,
) -> Vec<String> {
    let mut failures = mislabelled_anchor(&baseline.identity, identity);
    // The checkpoint is a manifest row an edit can move after the
    // anchor was recorded. A stop condition the run never reaches
    // leaves the steps, the outcome and the witnesses intact, so
    // nothing else in this comparison sees the move.
    let recorded_checkpoint = crate::paths::checkpoint_kind(checkpoint);
    if recorded_checkpoint != baseline.checkpoint {
        failures.push(format!(
            "checkpoint {} != recorded {}",
            recorded_checkpoint.as_markdown_label(),
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
    // Display, not Debug: `BootOutcome`'s `FromStr` round-trips the
    // Display form, and the two disagree for `PcReached`, whose Debug
    // prints the address in decimal.
    let recorded_outcome = baseline.outcome.to_string();
    if outcome != recorded_outcome {
        failures.push(format!("outcome {outcome} != recorded {recorded_outcome}"));
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
    failures
}

/// What one measured run of the set produced, in the terms the anchor
/// records.
pub(super) struct MeasuredRun<'a> {
    /// Stop condition the run was taken at.
    pub(super) checkpoint: manifest::CheckpointTrigger,
    pub(super) steps: u64,
    pub(super) budget: Budget,
    pub(super) outcome: String,
    /// The measuring process's own stderr: the witness lines, and the
    /// `RUN_IDENTITY` line naming what it composed.
    pub(super) stderr: &'a str,
}

/// Load the anchor for one cell of `content_id` and compare `run`
/// against it.
///
/// An unreadable or unparseable anchor is a disagreement, not a skip:
/// only a genuinely absent file means "nothing recorded yet".
///
/// [`workspace_root`] is compiled in, so a binary invoked outside the
/// tree it was built from reaches no anchor at all. That says nothing
/// about what is recorded, so it reports as
/// [`AnchorVerdict::NotComparable`] rather than letting every cell
/// look unrecorded.
pub(super) fn check_anchor(
    content_id: &str,
    cell: &CellKey,
    run: &MeasuredRun<'_>,
) -> AnchorVerdict {
    check_anchor_under(&workspace_root(), content_id, cell, run)
}

fn check_anchor_under(
    root: &Path,
    content_id: &str,
    cell: &CellKey,
    run: &MeasuredRun<'_>,
) -> AnchorVerdict {
    if !root.is_dir() {
        return AnchorVerdict::NotComparable(vec![format!(
            "the compiled-in workspace root {} is not present on this machine, so no \
             committed anchor is reachable",
            root.display()
        )]);
    }
    let path = boot_anchor_path(root, content_id, cell);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return AnchorVerdict::NotRecorded(cell.label())
        }
        Err(e) => {
            return AnchorVerdict::Drift(vec![format!("read {}: {e}", path.display())]);
        }
    };
    let baseline: BootSummary = match serde_json::from_str(&text) {
        Ok(b) => b,
        Err(e) => {
            return AnchorVerdict::Drift(vec![format!("parse {}: {e}", path.display())]);
        }
    };
    let observed = match parse_witness_lines(run.stderr) {
        Ok(w) => w,
        Err(errs) => {
            return AnchorVerdict::Drift(
                errs.iter()
                    .map(|e| format!("malformed witness line: {e}"))
                    .collect(),
            );
        }
    };
    // The identity triple comes from the measuring child's own stream.
    // The steps and the witnesses below come from that same stream. An
    // identity triple the parent resolved would hide the mismatch this
    // comparison exists to catch.
    let identity = match RunIdentity::parse_sentinel_lines(run.stderr) {
        Ok(Some(i)) => i,
        Ok(None) => {
            return AnchorVerdict::Drift(vec![format!(
                "the measured run printed no {RUN_IDENTITY_SENTINEL} line, so nothing says \
                 which firmware and title version produced these numbers"
            )])
        }
        Err(e) => return AnchorVerdict::Drift(vec![e.to_string()]),
    };
    let failures = anchor_disagreements(
        &baseline,
        &identity,
        run.checkpoint,
        run.steps,
        run.budget,
        &run.outcome,
        &observed,
    );
    if failures.is_empty() {
        AnchorVerdict::Match
    } else {
        AnchorVerdict::Drift(failures)
    }
}

#[cfg(test)]
#[path = "tests/anchor_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/anchor_override_tests.rs"]
mod override_tests;
