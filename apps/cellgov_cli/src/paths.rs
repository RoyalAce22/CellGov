//! Where a cell's committed artifacts live.
//!
//! `dev record-anchors` writes the boot anchors and `boot bench` gates
//! against them. They must agree on the cell an anchor is filed under;
//! the cap and checkpoint it is measured at are
//! `TitleManifest::cell_max_steps` and `TitleManifest::cell_checkpoint`.
//!
//! A boot anchor is not a scenario observation: anchors are CellGov's
//! own witnesses for a real title, under `tests/fixtures/<id>/`, while
//! `tests/scenario_observations/` holds RPCS3's answers for synthetic
//! scenarios.

use std::path::{Path, PathBuf};

use cellgov_boot::manifest::CellKey;

/// The cross-runner verdict `dev fixture-gen` writes and `dev
/// titles-gen` renders from.
pub(crate) const CROSS_RUNNER_SUMMARY_FILE: &str = "cross_runner_summary.json";

/// The committed fixture tree, relative to the workspace root.
pub(crate) const DEFAULT_FIXTURES_DIR: &str = "tests/fixtures";

/// Compiled-in workspace root.
///
/// A binary invoked from outside its build tree cannot reach the
/// fixtures; callers that gate on an anchor must probe the returned
/// path rather than treating a miss as "nothing recorded".
pub(crate) fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub(crate) fn fixtures_dir(root: &Path) -> PathBuf {
    root.join("tests").join("fixtures")
}

/// Directory holding one cell's committed anchor, under a fixture tree.
pub(crate) fn cell_anchor_dir_in(fixtures: &Path, content_id: &str, cell: &CellKey) -> PathBuf {
    cell_dir(
        fixtures.join(content_id).join("cellgov").join("anchors"),
        cell,
    )
}

/// Directory holding one cell's committed cross-runner triple, under a
/// fixture tree.
///
/// The cross-runner triple is `compare_report.txt`,
/// `cross_runner_summary.json` and `REPRODUCTION.md`, beside the
/// hand-maintained `NOTES.md`.
pub(crate) fn cell_cross_runner_dir_in(
    fixtures: &Path,
    content_id: &str,
    cell: &CellKey,
) -> PathBuf {
    cell_dir(fixtures.join(content_id).join("cross_runner"), cell)
}

/// The `fw-<ver>/<game-ver>` tail every per-cell artifact directory
/// ends in.
///
/// A firmware-shipped title has no game-version axis, so its cells sit
/// one level shallower.
fn cell_dir(base: PathBuf, cell: &CellKey) -> PathBuf {
    let dir = base.join(format!("fw-{}", cell.fw));
    match &cell.game_ver {
        Some(v) => dir.join(v),
        None => dir,
    }
}

/// Committed boot anchor for one cell of `content_id`, under a fixture
/// tree.
pub(crate) fn boot_anchor_path_in(fixtures: &Path, content_id: &str, cell: &CellKey) -> PathBuf {
    cell_anchor_dir_in(fixtures, content_id, cell).join("boot_summary.json")
}

/// Committed cross-runner summary for one cell of `content_id`, under
/// a fixture tree.
pub(crate) fn cross_runner_summary_path_in(
    fixtures: &Path,
    content_id: &str,
    cell: &CellKey,
) -> PathBuf {
    cell_cross_runner_dir_in(fixtures, content_id, cell).join(CROSS_RUNNER_SUMMARY_FILE)
}

/// Committed boot anchor for one cell of `content_id`, under a
/// workspace root.
pub(crate) fn boot_anchor_path(root: &Path, content_id: &str, cell: &CellKey) -> PathBuf {
    boot_anchor_path_in(&fixtures_dir(root), content_id, cell)
}

/// Append-only record of every anchor recorded for this cell.
pub(crate) fn history_path(root: &Path, content_id: &str, cell: &CellKey) -> PathBuf {
    cell_anchor_dir_in(&fixtures_dir(root), content_id, cell).join("boot_history.jsonl")
}

#[cfg(test)]
#[path = "tests/paths_tests.rs"]
mod tests;
