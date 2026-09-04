//! Where a cell's anchor lives, and the two parameters it is measured
//! under.
//!
//! `dev record-anchors` writes the boot anchors and `boot bench` gates
//! against them. They must agree on the cell an anchor is filed under,
//! and on the cap and checkpoint it was measured at.
//!
//! A boot anchor is not a scenario observation: anchors are CellGov's
//! own witnesses for a real title, under `tests/fixtures/<id>/`, while
//! `tests/scenario_observations/` holds RPCS3's answers for synthetic
//! scenarios.

use std::path::{Path, PathBuf};

use cellgov_compare::CheckpointKind;

use crate::game::manifest::{CellKey, CheckpointTrigger, MatrixCell, TitleManifest};

/// Instruction cap a cell is measured under when neither it nor its
/// title declares one.
pub(crate) const DEFAULT_BENCH_MAX_STEPS: u64 = 100_000_000;

/// Instruction cap `cell` is recorded and gated under.
pub(crate) fn cell_max_steps(title: &TitleManifest, cell: Option<&MatrixCell>) -> u64 {
    cell.and_then(|c| c.bench_max_steps)
        .or(title.bench_max_steps)
        .unwrap_or(DEFAULT_BENCH_MAX_STEPS)
}

/// Checkpoint `cell` is recorded and gated under.
pub(crate) fn cell_checkpoint(
    title: &TitleManifest,
    cell: Option<&MatrixCell>,
) -> CheckpointTrigger {
    cell.and_then(|c| c.checkpoint)
        .unwrap_or_else(|| title.checkpoint_trigger())
}

/// The wire form an anchor records a checkpoint in.
pub(crate) fn checkpoint_kind(cp: CheckpointTrigger) -> CheckpointKind {
    match cp {
        CheckpointTrigger::ProcessExit => CheckpointKind::ProcessExit,
        CheckpointTrigger::FirstRsxWrite => CheckpointKind::FirstRsxWrite,
        CheckpointTrigger::Pc(addr) => CheckpointKind::Pc {
            addr: cellgov_mem::GuestAddr::new(addr),
        },
    }
}

/// Compiled-in workspace root.
///
/// A binary invoked from outside its build tree cannot reach the
/// fixtures; callers that gate on an anchor must probe the returned
/// path rather than treating a miss as "nothing recorded".
pub(crate) fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root is two levels above apps/cellgov_cli")
        .to_path_buf()
}

pub(crate) fn fixtures_dir(root: &Path) -> PathBuf {
    root.join("tests").join("fixtures")
}

/// Directory holding one cell's committed anchor, under a fixture tree.
///
/// A firmware-shipped title has no game-version axis, so its cells sit
/// one level shallower.
pub(crate) fn cell_anchor_dir_in(fixtures: &Path, content_id: &str, cell: &CellKey) -> PathBuf {
    let dir = fixtures
        .join(content_id)
        .join("cellgov")
        .join("anchors")
        .join(format!("fw-{}", cell.fw));
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
