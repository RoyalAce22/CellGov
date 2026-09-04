//! The size walk behind `status`.

use std::path::Path;

/// What a size walk found, and what it did not count.
///
/// `bytes` is a floor whenever `unreadable` is non-zero.
#[derive(Debug, Default)]
pub(super) struct TreeSize {
    /// Total bytes of the regular files the walk read.
    pub bytes: u64,
    /// Entries the walk did not count in `bytes`.
    ///
    /// - a directory or entry it could not read
    /// - an entry that is neither a directory nor a regular file
    pub unreadable: usize,
}

/// Bytes every regular file under `dir` holds, and the paths refused.
///
/// An absent directory holds nothing; one that refuses for any other
/// reason counts as unreadable.
pub(super) fn tree_bytes(dir: &Path) -> TreeSize {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return TreeSize::default(),
        Err(_) => {
            return TreeSize {
                bytes: 0,
                unreadable: 1,
            }
        }
    };
    let mut total = TreeSize::default();
    for entry in entries {
        let Ok(entry) = entry else {
            total.unreadable += 1;
            continue;
        };
        let Ok(meta) = entry.metadata() else {
            total.unreadable += 1;
            continue;
        };
        if meta.is_dir() {
            let child = tree_bytes(&entry.path());
            total.bytes += child.bytes;
            total.unreadable += child.unreadable;
        } else if meta.is_file() {
            total.bytes += meta.len();
        } else {
            // `DirEntry::metadata` does not traverse links, so a
            // symlink -- and any socket, FIFO, or device node -- is
            // neither a directory nor a regular file. The walk does not
            // follow it, so its bytes are missing from `bytes`.
            total.unreadable += 1;
        }
    }
    total
}
