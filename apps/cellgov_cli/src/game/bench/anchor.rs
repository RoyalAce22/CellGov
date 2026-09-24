//! Whether one invocation can be held against its cell's committed
//! anchor, and where that anchor lives.

use std::path::Path;

use cellgov_compare::bench::{hold_against_anchor, load_anchor, AnchorVerdict, MeasuredRun};

use super::options::BenchOptions;
use crate::paths::{boot_anchor_path, workspace_root};
use cellgov_boot::manifest::CellKey;

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

/// Load the anchor for one cell of `content_id` and hold `run` against
/// it.
///
/// When the check reaches no usable anchor, the cause sets the verdict:
///
/// - an absent anchor file is [`AnchorVerdict::NotRecorded`];
/// - an unreadable or unparseable anchor is [`AnchorVerdict::Drift`];
/// - a missing compiled-in [`workspace_root`] is
///   [`AnchorVerdict::NotComparable`].
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
    let baseline = match load_anchor(&boot_anchor_path(root, content_id, cell)) {
        Ok(Some(b)) => b,
        Ok(None) => return AnchorVerdict::NotRecorded(cell.label()),
        Err(e) => return AnchorVerdict::Drift(vec![e.to_string()]),
    };
    let failures = hold_against_anchor(&baseline, run);
    if failures.is_empty() {
        AnchorVerdict::Match
    } else {
        AnchorVerdict::Drift(failures)
    }
}

#[cfg(test)]
#[path = "tests/anchor_tests.rs"]
mod tests;
