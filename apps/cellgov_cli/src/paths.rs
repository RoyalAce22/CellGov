//! Workspace-relative locations the anchor machinery shares.
//!
//! `record-anchors` writes the boot anchors and `bench-boot` gates
//! against them. They must agree on where an anchor lives and on the
//! instruction cap it was measured under -- a silent disagreement
//! there would make the gate compare a run against the wrong file, or
//! hold a default run against an anchor recorded at another cap.
//!
//! A boot anchor is not a scenario observation: anchors are CellGov's
//! own witnesses for a real title, under `tests/fixtures/<id>/`, while
//! `tests/scenario_observations/` holds RPCS3's answers for synthetic
//! scenarios.

use std::path::{Path, PathBuf};

/// Instruction cap a title boots under when its manifest sets none.
///
/// `record-anchors` measures with it, and `bench-boot` reads it to
/// tell a default-parameter run from one the operator retargeted.
pub(crate) const DEFAULT_BENCH_MAX_STEPS: u64 = 100_000_000;

/// Instruction cap the anchor for `title` is recorded and gated under.
///
/// `record-anchors` measures with it and `bench-boot` must default to
/// it: a bench run at any other cap is reported incomparable and gates
/// nothing, so a title that raises the cap would silently lose its
/// anchor check.
pub(crate) fn anchor_max_steps(title: &crate::game::manifest::TitleManifest) -> u64 {
    title.bench_max_steps.unwrap_or(DEFAULT_BENCH_MAX_STEPS)
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

/// Committed boot anchor for `content_id`, under `root`.
pub(crate) fn boot_anchor_path(root: &Path, content_id: &str) -> PathBuf {
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
