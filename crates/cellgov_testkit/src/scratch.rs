//! Scratch directories for filesystem tests.
//!
//! This module is the workspace's only source of temporary test paths.
//! `scratch_dir_guard` refuses a raw `std::env::temp_dir()` call
//! elsewhere under `apps/`, `bridges/` and `crates/`, except at the
//! sites its own allowance table names.
//!
//! A [`ScratchDir`] removes its tree when it drops, including while a
//! panic unwinds, so a red test leaves nothing behind. Each name
//! carries a random suffix, so two runs of one test never resolve to
//! the same path.

use std::ops::Deref;
use std::path::Path;

use tempfile::{Builder, TempDir};

/// Keeps every scratch directory after its test ends, and prints where.
const RETAIN_VAR: &str = "CELLGOV_RETAIN_SCRATCH";

/// Prefix every scratch directory carries, so a retained tree is
/// attributable to this workspace.
const PREFIX: &str = "cellgov_scratch_";

/// A directory that removes itself when it drops.
pub struct ScratchDir {
    /// `None` only after [`Drop`] takes the directory.
    dir: Option<TempDir>,
}

impl Deref for ScratchDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        self.dir
            .as_ref()
            .expect("invariant: the directory is taken only by Drop")
            .path()
    }
}

impl AsRef<Path> for ScratchDir {
    fn as_ref(&self) -> &Path {
        self
    }
}

impl Drop for ScratchDir {
    #[allow(
        clippy::print_stderr,
        reason = "a drop-time cleanup refusal cannot panic; stderr is the only channel left"
    )]
    fn drop(&mut self) {
        let Some(dir) = self.dir.take() else {
            return;
        };
        if std::env::var_os(RETAIN_VAR).is_some() {
            // `keep` releases the directory without removing it.
            let path = dir.keep();
            eprintln!("{RETAIN_VAR}: retained {}", path.display());
            return;
        }
        // `TempDir`'s own `Drop` discards the removal error; `close`
        // returns it. A panic out of `drop` aborts the process, so this
        // prints the refusal on stderr.
        let path = dir.path().to_path_buf();
        if let Err(e) = dir.close() {
            eprintln!("scratch dir {} not removed on drop: {e}", path.display());
        }
    }
}

/// A fresh, empty scratch directory.
///
/// # Panics
///
/// Panics if the directory cannot be created.
#[must_use]
pub fn scratch() -> ScratchDir {
    build(PREFIX.to_string())
}

/// A fresh, empty scratch directory whose name carries `label`.
///
/// Every character of `label` outside ASCII alphanumerics folds to `_`.
///
/// # Panics
///
/// Panics if the directory cannot be created.
#[must_use]
pub fn scratch_labeled(label: &str) -> ScratchDir {
    let safe: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    build(format!("{PREFIX}{safe}_"))
}

fn build(prefix: String) -> ScratchDir {
    let dir = Builder::new()
        .prefix(&prefix)
        .tempdir()
        .unwrap_or_else(|e| panic!("scratch dir under {prefix:?} not creatable: {e}"));
    ScratchDir { dir: Some(dir) }
}

#[cfg(test)]
#[path = "tests/scratch_tests.rs"]
mod tests;
