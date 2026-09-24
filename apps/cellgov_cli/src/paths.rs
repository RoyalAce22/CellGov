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
pub(crate) use cellgov_boot::manifest::{
    boot_anchor_path_in, cell_anchor_dir_in, cell_cross_runner_dir_in,
    cross_runner_summary_path_in, CROSS_RUNNER_SUMMARY_FILE,
};

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
