//! The drift tests' write mode: `CELLGOV_REGENERATE=1` makes each one
//! write the file it checks instead of comparing it.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Whether `CELLGOV_REGENERATE=1` asks the drift tests to write.
pub(crate) fn asked() -> bool {
    std::env::var_os("CELLGOV_REGENERATE").is_some_and(|v| v == "1")
}

/// Writes `contents` to `path` and returns `true` when [`asked`];
/// otherwise returns `false`. The write goes through a sibling file and
/// a rename, so a test that reads `path` at the same time reads the old
/// file or the new one. Each write stages under its own name, so two
/// tests that write one path do not take each other's staged file.
pub(crate) fn write_if_asked(path: &Path, contents: impl AsRef<[u8]>) -> bool {
    static STAGED: AtomicUsize = AtomicUsize::new(0);
    if !asked() {
        return false;
    }
    let n = STAGED.fetch_add(1, Ordering::Relaxed);
    let staged = path.with_extension(format!("regenerate-{}-{n}", std::process::id()));
    std::fs::write(&staged, contents).unwrap_or_else(|e| panic!("write {}: {e}", staged.display()));
    std::fs::rename(&staged, path)
        .unwrap_or_else(|e| panic!("rename {} to {}: {e}", staged.display(), path.display()));
    true
}
