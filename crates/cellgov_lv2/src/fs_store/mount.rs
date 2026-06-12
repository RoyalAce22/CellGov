use std::path::PathBuf;

use super::FsError;

/// One read-only mount: a guest-path prefix served from a host
/// directory.
///
/// `prefix` is normalized at construction (no trailing `/`, must
/// start with `/`). Writes / mkdir / unlink return CELL_EROFS from
/// the dispatch layer regardless of host-side permissions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsMount {
    /// Guest path prefix, e.g. `/app_home`. No trailing slash.
    pub prefix: String,
    /// Host directory backing the mount.
    pub host_root: PathBuf,
}

impl FsMount {
    /// Build a mount, normalizing `prefix`.
    ///
    /// # Errors
    ///
    /// Returns `None` if `prefix` is empty, doesn't start with `/`,
    /// or contains `..`.
    pub fn new(prefix: impl Into<String>, host_root: PathBuf) -> Option<Self> {
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
        Some(Self { prefix, host_root })
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

    /// Resolve a guest path to a host path.
    ///
    /// Normalizes empty segments (`//`) and `.` segments; rejects
    /// `..` segments as [`FsError::PathTraversal`].
    ///
    /// # Errors
    ///
    /// - [`FsError::PathTraversal`] when the resolved path would
    ///   escape the mount root via `..` segments.
    pub fn resolve(&self, guest_path: &str) -> Result<Option<PathBuf>, FsError> {
        for mount in &self.mounts {
            let Some(rest) = strip_mount_prefix(guest_path, &mount.prefix) else {
                continue;
            };
            let mut host = mount.host_root.clone();
            for segment in rest.split('/') {
                if segment.is_empty() || segment == "." {
                    continue;
                }
                if segment == ".." {
                    return Err(FsError::PathTraversal);
                }
                host.push(segment);
            }
            return Ok(Some(host));
        }
        Ok(None)
    }

    /// Iterate registered mounts in registration order.
    pub fn mounts(&self) -> impl Iterator<Item = &FsMount> {
        self.mounts.iter()
    }
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
