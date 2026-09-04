use std::path::{Path, PathBuf};

use super::FsError;

/// One read-only mount: a guest-path prefix served from an ordered
/// list of host roots.
///
/// `prefix` is normalized at construction (no trailing `/`, must
/// start with `/`). `roots` is never empty. Writes / mkdir / unlink
/// return CELL_EROFS from the dispatch layer regardless of host-side
/// permissions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsMount {
    /// Guest path prefix, e.g. `/app_home`. No trailing slash.
    pub prefix: String,
    roots: Vec<PathBuf>,
}

impl FsMount {
    /// Build a mount with one root.
    ///
    /// # Errors
    ///
    /// Returns `None` if `prefix` is empty, doesn't start with `/`,
    /// or contains `..`.
    pub fn new(prefix: impl Into<String>, host_root: PathBuf) -> Option<Self> {
        Self::with_roots(prefix, vec![host_root])
    }

    /// Build a mount served from `roots` in probe order.
    ///
    /// An earlier root shadows a later one.
    ///
    /// # Errors
    ///
    /// Returns `None` when:
    ///
    /// - `roots` is empty.
    /// - `prefix` is empty, or does not start with `/`.
    /// - `prefix` contains a `..` segment.
    pub fn with_roots(prefix: impl Into<String>, roots: Vec<PathBuf>) -> Option<Self> {
        if roots.is_empty() {
            return None;
        }
        let mut prefix = prefix.into();
        if prefix.is_empty() || !prefix.starts_with('/') {
            return None;
        }
        if prefix.split('/').any(|seg| seg == "..") {
            return None;
        }
        while prefix.len() > 1 && prefix.ends_with('/') {
            prefix.pop();
        }
        Some(Self { prefix, roots })
    }

    /// Host roots in probe order. Never empty.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }
}

/// Ordered set of [`FsMount`]s.
///
/// Mounts are consulted in registration order; the first whose
/// prefix matches resolves the path.
#[derive(Debug, Clone, Default)]
pub struct FsMountTable {
    mounts: Vec<FsMount>,
}

impl FsMountTable {
    /// Empty mount table.
    pub fn new() -> Self {
        Self::default()
    }

    /// # Errors
    ///
    /// - [`FsError::MountAlreadyRegistered`] if a mount with the
    ///   same prefix is already in the table.
    pub fn add(&mut self, mount: FsMount) -> Result<(), FsError> {
        if self.mounts.iter().any(|m| m.prefix == mount.prefix) {
            return Err(FsError::MountAlreadyRegistered);
        }
        self.mounts.push(mount);
        Ok(())
    }

    /// Resolve a guest path to one host path per root of the first
    /// matching mount, in probe order.
    ///
    /// This reads no host directory. The candidate list is a function
    /// of the guest path and the root list, so the caller decides
    /// which candidate is a hit. Empty (`//`) and `.` segments drop
    /// out.
    ///
    /// # Errors
    ///
    /// - [`FsError::PathTraversal`] for a `..` segment, or for a
    ///   segment that carries `\` or `:`. Win32 reads them as a
    ///   separator and a drive/stream marker, so the join would leave
    ///   the root. A POSIX host reads them as filename characters.
    ///   CellGov refuses them there too, so the result never depends
    ///   on the host OS.
    pub fn resolve_candidates(&self, guest_path: &str) -> Result<Option<Vec<PathBuf>>, FsError> {
        for mount in &self.mounts {
            let Some(rest) = strip_mount_prefix(guest_path, &mount.prefix) else {
                continue;
            };
            let segments = split_segments(rest)?;
            let candidates = mount
                .roots
                .iter()
                .map(|root| join_segments(root, &segments))
                .collect();
            return Ok(Some(candidates));
        }
        Ok(None)
    }

    /// Iterate registered mounts in registration order.
    pub fn mounts(&self) -> impl Iterator<Item = &FsMount> {
        self.mounts.iter()
    }
}

/// Split a mount-relative path into segments, without the empty and
/// `.` segments.
///
/// [`FsMountTable::resolve_candidates`] states which segments this
/// refuses and why. `cellgov_install`'s `tree_rel_path_is_safe`
/// applies the same rule.
fn split_segments(rest: &str) -> Result<Vec<&str>, FsError> {
    let mut segments = Vec::new();
    for segment in rest.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." || segment.contains('\\') || segment.contains(':') {
            return Err(FsError::PathTraversal);
        }
        segments.push(segment);
    }
    Ok(segments)
}

fn join_segments(root: &Path, segments: &[&str]) -> PathBuf {
    let mut host = root.to_path_buf();
    for segment in segments {
        host.push(segment);
    }
    host
}

/// Match `guest_path` against `prefix`, succeeding on exact match
/// or `prefix + '/'`. The root mount `/` matches any path starting
/// with `/`, stripping the leading slash.
fn strip_mount_prefix<'a>(guest_path: &'a str, prefix: &str) -> Option<&'a str> {
    if guest_path == prefix {
        return Some("");
    }
    if prefix == "/" {
        return guest_path.strip_prefix('/');
    }
    let with_slash_len = prefix.len() + 1;
    if guest_path.len() >= with_slash_len
        && guest_path.starts_with(prefix)
        && guest_path.as_bytes()[prefix.len()] == b'/'
    {
        Some(&guest_path[with_slash_len..])
    } else {
        None
    }
}

#[cfg(test)]
#[path = "tests/mount_tests.rs"]
mod tests;
