//! Workspace-relative locations the anchor machinery shares.
//!
//! `record-anchors` writes the baselines and `bench-boot` gates
//! against them. They must agree on where a baseline lives and on the
//! instruction cap it was measured under -- a silent disagreement
//! there would make the gate compare a run against the wrong file, or
//! hold a default run against an anchor recorded at another cap.

use std::path::{Path, PathBuf};

/// Instruction cap a title boots under when its manifest sets none.
///
/// `record-anchors` measures with it, and `bench-boot` reads it to
/// tell a default-parameter run from one the operator retargeted.
pub(crate) const DEFAULT_BENCH_MAX_STEPS: u64 = 100_000_000;

/// Compiled-in workspace root.
///
/// A binary invoked from outside its build tree cannot reach the
/// fixtures; callers that gate on a baseline must probe the returned
/// path rather than treating a miss as "nothing recorded".
pub(crate) fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root is two levels above apps/cellgov_cli")
        .to_path_buf()
}

/// Committed boot anchor for `content_id`, under `root`.
pub(crate) fn baseline_path(root: &Path, content_id: &str) -> PathBuf {
    root.join(format!(
        "tests/fixtures/{content_id}/cellgov/boot_summary.json"
    ))
}

/// Append-only record of every anchor this title has been recorded at.
pub(crate) fn history_path(root: &Path, content_id: &str) -> PathBuf {
    root.join(format!(
        "tests/fixtures/{content_id}/cellgov/boot_history.jsonl"
    ))
}
