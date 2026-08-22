//! Per-process scratch directories for filesystem tests. Declared by
//! both `lib.rs` and `main.rs` so the library and binary test suites
//! share one implementation. Compiled only under `cfg(test)`.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static SCRATCH_SEQ: AtomicU32 = AtomicU32::new(0);

/// A scratch directory, removed recursively on drop.
///
/// The path carries the process id and a per-call counter, so neither
/// overlapping `cargo test` / `cargo test --release` invocations nor
/// two tests running on parallel harness threads resolve to the same
/// directory. Drop runs on unwind, so a failing assertion cleans up;
/// a drop-time removal that the OS refuses is reported on stderr,
/// since a panic there would abort the run.
pub struct ScratchDir {
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
            // Unwinding out of `drop` aborts the process, so a refusal
            // is named rather than raised. Leftovers are harmless --
            // the next run with this pid sweeps them -- but silence
            // would hide an open-handle or permission problem.
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
/// If a stale directory at the same path cannot be removed, or the
/// fresh one cannot be created.
pub fn scratch() -> ScratchDir {
    let n = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "cellgov_install_scratch_{}_{}",
        std::process::id(),
        n
    ));
    // A pid is reused only once the earlier process is gone, so this
    // sweeps a crashed run's leftovers without racing a live one.
    // Anything other than "it was not there" means the fresh directory
    // would inherit that run's files, so it is a hard failure.
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
#[path = "tests/scratch_dir_tests.rs"]
mod tests;
