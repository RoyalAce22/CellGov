//! The step count a boot's progress bar counts down to.
//!
//! The step cap is not the finish line: a boot almost never reaches
//! it, so a ratio against it never leaves its first decile. The bar
//! counts down to:
//!
//! - the step count the cell's committed anchor recorded, when this
//!   run retraces the anchor's trajectory;
//! - the cap, when it is below the anchor, since the run ends there;
//! - nothing, otherwise: the bar counts steps and predicts nothing.

use std::path::{Path, PathBuf};

use cellgov_compare::BootSummary;

use crate::paths::{boot_anchor_path, workspace_root};
use cellgov_boot::manifest::CellKey;

/// Why a committed anchor gave no step count.
#[derive(Debug, thiserror::Error)]
enum AnchorReadError {
    #[error("read {}: {source}", .path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parse {}: {source}", .path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

/// The step count `cell`'s committed anchor recorded.
///
/// Answers `None` when there is nothing to predict:
///
/// - `cell` is `None`;
/// - `retargeted`: an override moved the run off the anchor's trajectory;
/// - this binary can read no anchor.
///
/// An absent anchor is quiet. An anchor that is present and unreadable
/// or malformed also answers `None`, and reports itself on stderr.
pub(crate) fn anchor_finish_line(
    content_id: &str,
    cell: Option<&CellKey>,
    retargeted: bool,
) -> Option<u64> {
    if retargeted {
        return None;
    }
    anchor_steps_under(&workspace_root(), content_id, cell?)
}

/// The finish line a run can reach: `finish_line`, or `cap_remaining`,
/// the loop steps the cap still allows, when that is fewer.
///
/// `boot run`'s default cap sits under every committed anchor, so a
/// line at the anchor would predict a moment past the cap. A spent cap
/// leaves nothing to count down to.
pub(crate) fn within_cap(finish_line: Option<u64>, cap_remaining: u64) -> Option<u64> {
    finish_line
        .map(|steps| steps.min(cap_remaining))
        .filter(|&steps| steps > 0)
}

/// [`within_cap`] against `rt`'s own cap: the step calls it has left
/// before it refuses the next one.
pub(crate) fn within_runtime_cap(
    finish_line: Option<u64>,
    rt: &cellgov_core::Runtime,
) -> Option<u64> {
    within_cap(
        finish_line,
        rt.max_steps().saturating_sub(rt.steps_taken()) as u64,
    )
}

fn anchor_steps_under(root: &Path, content_id: &str, cell: &CellKey) -> Option<u64> {
    match read_anchor_steps(&boot_anchor_path(root, content_id, cell)) {
        Ok(steps) => Some(steps),
        // The cell has no anchor, or the binary runs outside the tree
        // it was built in. The anchor gate reads the same absence as
        // "not recorded".
        Err(AnchorReadError::Read { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            None
        }
        // `boot bench` holds a malformed anchor against the gate;
        // `boot run` and a direct `boot bench-once` have no gate, so
        // this line is their only report of it.
        Err(e) => {
            eprintln!("anchor: {e}; the bar counts with no finish line");
            None
        }
    }
}

fn read_anchor_steps(path: &Path) -> Result<u64, AnchorReadError> {
    let text = std::fs::read_to_string(path).map_err(|source| AnchorReadError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let summary: BootSummary =
        serde_json::from_str(&text).map_err(|source| AnchorReadError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(summary.steps)
}

#[cfg(test)]
#[path = "tests/finish_line_tests.rs"]
mod tests;
