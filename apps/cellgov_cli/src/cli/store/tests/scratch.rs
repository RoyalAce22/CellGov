//! Per-process scratch directories for the store commands' filesystem
//! tests.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static SCRATCH_SEQ: AtomicU32 = AtomicU32::new(0);

/// A scratch directory, removed recursively on drop.
///
/// The path carries the process id and a per-call counter. Two tests on
/// parallel harness threads never share a path, and neither do
/// overlapping `cargo test` and `cargo test --release` runs.
pub(crate) struct ScratchDir {
    path: PathBuf,
}

impl std::ops::Deref for ScratchDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.path) {
            // A panic in `drop` aborts the process, so report the
            // refusal on stderr instead.
            eprintln!(
                "scratch dir {} not removed on drop: {e}",
                self.path.display()
            );
        }
    }
}

/// A fresh, empty scratch directory unique to this process and call.
///
/// # Panics
///
/// Panics when a stale directory at the same path cannot be removed, or
/// when the fresh one cannot be created.
pub(crate) fn scratch() -> ScratchDir {
    let n = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("cellgov_store_{}_{n}", std::process::id()));
    // A pid is reused only once the earlier process is gone, so this
    // sweeps a crashed run's leftovers without racing a live one.
    match std::fs::remove_dir_all(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => panic!("stale scratch dir {} not removable: {e}", path.display()),
    }
    std::fs::create_dir_all(&path)
        .unwrap_or_else(|e| panic!("scratch dir {} not creatable: {e}", path.display()));
    ScratchDir { path }
}

#[cfg(test)]
#[path = "scratch_tests.rs"]
mod tests;
