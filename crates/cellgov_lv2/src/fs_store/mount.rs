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
mod tests {
    use super::*;

    fn standard_table() -> FsMountTable {
        let mut t = FsMountTable::new();
        t.add(FsMount::new("/app_home", PathBuf::from("/host/app")).unwrap())
            .unwrap();
        t.add(FsMount::new("/dev_hdd0", PathBuf::from("/host/hdd0")).unwrap())
            .unwrap();
        t
    }

    #[test]
    fn resolve_simple_app_home() {
        let t = standard_table();
        assert_eq!(
            t.resolve("/app_home/Data/first.xml").unwrap(),
            Some(PathBuf::from("/host/app/Data/first.xml"))
        );
    }

    #[test]
    fn resolve_strips_dot_segments() {
        let t = standard_table();
        assert_eq!(
            t.resolve("/app_home/./Data/./first.xml").unwrap(),
            Some(PathBuf::from("/host/app/Data/first.xml"))
        );
    }

    #[test]
    fn resolve_collapses_double_slashes() {
        let t = standard_table();
        assert_eq!(
            t.resolve("/app_home//Data//first.xml").unwrap(),
            Some(PathBuf::from("/host/app/Data/first.xml"))
        );
    }

    #[test]
    fn resolve_rejects_dotdot_traversal() {
        let t = standard_table();
        assert_eq!(
            t.resolve("/app_home/../etc/passwd"),
            Err(FsError::PathTraversal)
        );
        assert_eq!(
            t.resolve("/app_home/Data/../../etc/passwd"),
            Err(FsError::PathTraversal)
        );
    }

    #[test]
    fn resolve_returns_none_for_no_mount() {
        let t = standard_table();
        assert_eq!(t.resolve("/dev_flash/foo").unwrap(), None);
    }

    #[test]
    fn resolve_handles_exact_prefix_match() {
        let t = standard_table();
        assert_eq!(
            t.resolve("/app_home").unwrap(),
            Some(PathBuf::from("/host/app"))
        );
    }

    #[test]
    fn resolve_handles_prefix_with_trailing_slash() {
        let t = standard_table();
        assert_eq!(
            t.resolve("/app_home/").unwrap(),
            Some(PathBuf::from("/host/app"))
        );
    }

    #[test]
    fn resolve_partial_prefix_does_not_match() {
        let t = standard_table();
        assert_eq!(t.resolve("/app_homeFoo").unwrap(), None);
        assert_eq!(t.resolve("/app_homeFoo/bar").unwrap(), None);
    }

    #[test]
    fn resolve_picks_first_matching_mount() {
        let mut t = FsMountTable::new();
        t.add(FsMount::new("/app_home", PathBuf::from("/first")).unwrap())
            .unwrap();
        t.add(FsMount::new("/app_home_alt", PathBuf::from("/second")).unwrap())
            .unwrap();
        assert_eq!(
            t.resolve("/app_home/x").unwrap(),
            Some(PathBuf::from("/first/x"))
        );
        assert_eq!(
            t.resolve("/app_home_alt/x").unwrap(),
            Some(PathBuf::from("/second/x"))
        );
    }

    #[test]
    fn add_rejects_duplicate_prefix() {
        let mut t = FsMountTable::new();
        t.add(FsMount::new("/app_home", PathBuf::from("/a")).unwrap())
            .unwrap();
        let err = t
            .add(FsMount::new("/app_home", PathBuf::from("/b")).unwrap())
            .unwrap_err();
        assert_eq!(err, FsError::MountAlreadyRegistered);
    }

    #[test]
    fn mount_new_normalizes_trailing_slash() {
        let m = FsMount::new("/app_home/", PathBuf::from("/x")).unwrap();
        assert_eq!(m.prefix, "/app_home");
    }

    #[test]
    fn mount_new_rejects_relative_prefix() {
        assert!(FsMount::new("app_home", PathBuf::from("/x")).is_none());
        assert!(FsMount::new("", PathBuf::from("/x")).is_none());
    }

    #[test]
    fn mount_new_rejects_dotdot_in_prefix() {
        assert!(FsMount::new("/app_home/..", PathBuf::from("/x")).is_none());
        assert!(FsMount::new("/../etc", PathBuf::from("/x")).is_none());
    }

    #[test]
    fn empty_table_resolves_nothing() {
        let t = FsMountTable::new();
        assert_eq!(t.resolve("/app_home/foo").unwrap(), None);
        assert_eq!(t.resolve("/").unwrap(), None);
    }

    #[test]
    fn mounts_iterates_in_registration_order() {
        let t = standard_table();
        let prefixes: Vec<&str> = t.mounts().map(|m| m.prefix.as_str()).collect();
        assert_eq!(prefixes, vec!["/app_home", "/dev_hdd0"]);
    }

    #[test]
    fn resolve_root_mount_with_subpath() {
        let mut t = FsMountTable::new();
        t.add(FsMount::new("/", PathBuf::from("/host")).unwrap())
            .unwrap();
        assert_eq!(t.resolve("/").unwrap(), Some(PathBuf::from("/host")));
        assert_eq!(
            t.resolve("/foo/bar").unwrap(),
            Some(PathBuf::from("/host/foo/bar"))
        );
    }
}
