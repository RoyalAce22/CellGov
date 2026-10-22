//! The one input file of a kind in an operator dump directory.
#![allow(
    dead_code,
    reason = "each integration-test binary compiles this module separately and uses a subset"
)]

use std::path::{Path, PathBuf};

/// The one file in `dir` whose extension matches `ext` (ASCII-insensitive).
///
/// # Panics
///
/// If `dir` is unreadable, or holds no such file, or holds more than
/// one. `read_dir` yields in host-defined order, so picking one of
/// several would make which package gets installed a property of the
/// filesystem.
pub fn only_with_ext(dir: &Path, ext: &str) -> PathBuf {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read dir {}: {e}", dir.display()));
    let mut hits: Vec<PathBuf> = entries
        .map(|e| {
            e.unwrap_or_else(|e| panic!("read an entry of {}: {e}", dir.display()))
                .path()
        })
        .filter(|p| {
            p.extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case(ext))
        })
        .collect();
    hits.sort();
    match hits.len() {
        1 => hits.remove(0),
        0 => panic!("no .{ext} in {}", dir.display()),
        _ => panic!(
            "{} holds {} .{ext} files ({}); leave exactly the one this \
             parity gate's digest was blessed against",
            dir.display(),
            hits.len(),
            hits.iter()
                .map(|p| p.file_name().unwrap_or_default().to_string_lossy())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
