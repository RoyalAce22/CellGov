//! Scratch directories for the install integration tests.
//!
//! These tests install whole firmware or game trees, so a leaked
//! directory costs gigabytes. Cleanup runs on drop, which covers a
//! failing assertion; the path carries the process id so overlapping
//! `cargo test` invocations stay out of each other's way.

// Each integration test binary compiles this module separately and
// uses a different subset of it.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static SCRATCH_SEQ: AtomicU32 = AtomicU32::new(0);

/// A scratch directory, removed recursively on drop.
pub struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    /// A fresh, empty scratch directory unique to this process and call.
    ///
    /// # Panics
    ///
    /// If a stale directory at the same path cannot be removed, or the
    /// fresh one cannot be created.
    pub fn new(label: &str) -> Self {
        let n = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "cellgov_install_it_{label}_{}_{n}",
            std::process::id()
        ));
        match std::fs::remove_dir_all(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("stale scratch dir {} not removable: {e}", path.display()),
        }
        std::fs::create_dir_all(&path)
            .unwrap_or_else(|e| panic!("scratch dir {} not creatable: {e}", path.display()));
        Self { path }
    }
}

impl std::ops::Deref for ScratchDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchDir {
    #[allow(
        clippy::print_stderr,
        reason = "a drop-time cleanup refusal cannot panic; stderr is the only channel left"
    )]
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.path) {
            eprintln!(
                "scratch dir {} not removed on drop: {e}",
                self.path.display()
            );
        }
    }
}
