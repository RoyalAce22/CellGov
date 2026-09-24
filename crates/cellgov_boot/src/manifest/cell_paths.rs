//! Where a cell's committed artifacts live under a fixture tree.
//!
//! `dev record-anchors` writes a cell's boot anchor and `boot bench`
//! gates against it; `dev fixture-gen` writes its cross-runner triple
//! and `dev titles-gen` renders from both. All of them, and the
//! installed-title suites, file a cell under the same directory.

use std::path::{Path, PathBuf};

use super::CellKey;

/// A cell's committed boot anchor file.
pub const BOOT_SUMMARY_FILE: &str = "boot_summary.json";

/// The cross-runner verdict `dev fixture-gen` writes and `dev
/// titles-gen` renders from.
pub const CROSS_RUNNER_SUMMARY_FILE: &str = "cross_runner_summary.json";

/// The directory holding every boot anchor of `content_id`.
pub fn title_anchors_dir_in(fixtures: &Path, content_id: &str) -> PathBuf {
    fixtures.join(content_id).join("cellgov").join("anchors")
}

/// The directory holding every cross-runner triple of `content_id`.
pub fn title_cross_runner_dir_in(fixtures: &Path, content_id: &str) -> PathBuf {
    fixtures.join(content_id).join("cross_runner")
}

/// The directory holding one cell's committed anchor.
pub fn cell_anchor_dir_in(fixtures: &Path, content_id: &str, cell: &CellKey) -> PathBuf {
    cell_dir(title_anchors_dir_in(fixtures, content_id), cell)
}

/// The directory holding one cell's committed cross-runner triple:
/// `compare_report.txt`, `cross_runner_summary.json` and
/// `REPRODUCTION.md`, beside the hand-maintained `NOTES.md`.
pub fn cell_cross_runner_dir_in(fixtures: &Path, content_id: &str, cell: &CellKey) -> PathBuf {
    cell_dir(title_cross_runner_dir_in(fixtures, content_id), cell)
}

/// One cell's committed boot anchor.
pub fn boot_anchor_path_in(fixtures: &Path, content_id: &str, cell: &CellKey) -> PathBuf {
    cell_anchor_dir_in(fixtures, content_id, cell).join(BOOT_SUMMARY_FILE)
}

/// One cell's committed cross-runner summary.
pub fn cross_runner_summary_path_in(fixtures: &Path, content_id: &str, cell: &CellKey) -> PathBuf {
    cell_cross_runner_dir_in(fixtures, content_id, cell).join(CROSS_RUNNER_SUMMARY_FILE)
}

/// The `fw-<ver>/<game-ver>` tail every per-cell artifact directory
/// ends in. A firmware-shipped title has no game-version axis, so its
/// cells sit one level shallower.
fn cell_dir(base: PathBuf, cell: &CellKey) -> PathBuf {
    let dir = base.join(format!("fw-{}", cell.fw));
    match &cell.game_ver {
        Some(v) => dir.join(v),
        None => dir,
    }
}

#[cfg(test)]
#[path = "tests/cell_paths_tests.rs"]
mod tests;
