//! Boot-time mount-table provider.
//!
//! Reads each `[[fs.mounts]]` entry, resolves the host directory
//! (with optional per-mount env-var override), and adds an
//! [`FsMount`] to [`Lv2Host::fs_mounts_mut`]. A missing or
//! non-directory host root is a startup error.
//!
//! # Validation order
//!
//! Per-entry: pure-shape validation (prefix and host string) runs
//! BEFORE any I/O or env lookup, so a multi-error manifest surfaces
//! shape problems before I/O problems.

use std::path::{Path, PathBuf};

use cellgov_lv2::{FsMount, Lv2Host};

use super::manifest::MountEntry;

/// Why [`register_mounts`] could not register a mount entry.
#[derive(Debug)]
pub enum MountRegisterError {
    /// Prefix failed shape validation (empty, non-`/`, `..` segment).
    InvalidPrefix { prefix: String, reason: String },
    /// Host string failed shape validation (empty, `..` segment,
    /// non-POSIX absolute shape, or empty `override_env` name).
    InvalidHost {
        prefix: String,
        host: String,
        reason: String,
    },
    /// Host root does not exist.
    HostRootMissing {
        prefix: String,
        host_path: PathBuf,
        /// Override env var name, surfaced in Display.
        override_env: Option<String>,
    },
    /// Host root exists but is not a directory.
    HostRootNotDirectory {
        prefix: String,
        host_path: PathBuf,
        override_env: Option<String>,
    },
    /// Host-root metadata probe failed (non-NotFound I/O error).
    HostRootIo {
        prefix: String,
        host_path: PathBuf,
        source: std::io::Error,
        override_env: Option<String>,
    },
    /// `FsMountTable::add` rejected the prefix.
    DuplicatePrefix { prefix: String },
}

impl std::fmt::Display for MountRegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPrefix { prefix, reason } => {
                write!(f, "mounts: prefix {prefix:?} is invalid: {reason}",)
            }
            Self::InvalidHost {
                prefix,
                host,
                reason,
            } => write!(
                f,
                "mounts: host {host:?} for prefix {prefix:?} is invalid: {reason}",
            ),
            Self::HostRootMissing {
                prefix,
                host_path,
                override_env,
            } => {
                write!(
                    f,
                    "mounts: prefix {:?} resolved to {} which does not exist",
                    prefix,
                    host_path.display(),
                )?;
                write_override_hint(f, override_env)
            }
            Self::HostRootNotDirectory {
                prefix,
                host_path,
                override_env,
            } => {
                write!(
                    f,
                    "mounts: prefix {:?} resolved to {} which exists but is not a directory",
                    prefix,
                    host_path.display(),
                )?;
                write_override_hint(f, override_env)
            }
            Self::HostRootIo {
                prefix,
                host_path,
                source,
                override_env,
            } => {
                write!(
                    f,
                    "mounts: prefix {:?} could not stat {}: {source}",
                    prefix,
                    host_path.display(),
                )?;
                write_override_hint(f, override_env)
            }
            Self::DuplicatePrefix { prefix } => write!(
                f,
                "mounts: prefix {prefix:?} is already registered (FsMountTable rejected it)",
            ),
        }
    }
}

fn write_override_hint(
    f: &mut std::fmt::Formatter<'_>,
    override_env: &Option<String>,
) -> std::fmt::Result {
    if let Some(env) = override_env {
        write!(
            f,
            " (override env var {env} is set; either point it at a real directory or \
             unset {env} to fall back to the manifest's checked-in host)",
        )?;
    }
    Ok(())
}

impl std::error::Error for MountRegisterError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::HostRootIo { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// POSIX-shape resolution: leading `/` is absolute on every host;
/// otherwise relative to `base`. Pure path arithmetic.
///
/// `Path::is_absolute` is NOT used -- on Windows `/abs/path` is
/// drive-relative, which would silently diverge from Linux and break
/// byte-identical replay. `validate_host_shape` pre-rejects Windows-shape
/// inputs so they never reach this resolver.
fn resolve_against(base: &Path, path: &str) -> PathBuf {
    if path.starts_with('/') {
        PathBuf::from(path)
    } else {
        base.join(path)
    }
}

/// Canonicalize an already-verified directory so symlink-divergent
/// checkouts produce the same `host_root` (byte-identical replay).
fn canonicalize_existing(
    prefix: &str,
    host_path: &Path,
    override_env: &Option<String>,
) -> Result<PathBuf, MountRegisterError> {
    std::fs::canonicalize(host_path).map_err(|source| MountRegisterError::HostRootIo {
        prefix: prefix.to_string(),
        host_path: host_path.to_path_buf(),
        source,
        override_env: override_env.clone(),
    })
}

/// Pure-shape prefix validation. Runs BEFORE any I/O or env lookup.
fn validate_prefix(prefix: &str) -> Result<(), MountRegisterError> {
    if prefix.is_empty() {
        return Err(MountRegisterError::InvalidPrefix {
            prefix: prefix.to_string(),
            reason: "prefix is empty".to_string(),
        });
    }
    if !prefix.starts_with('/') {
        return Err(MountRegisterError::InvalidPrefix {
            prefix: prefix.to_string(),
            reason: "prefix must start with '/'".to_string(),
        });
    }
    if prefix.split('/').any(|seg| seg == "..") {
        return Err(MountRegisterError::InvalidPrefix {
            prefix: prefix.to_string(),
            reason: "prefix must not contain '..' segments".to_string(),
        });
    }
    Ok(())
}

/// Pure-shape host-string validation. Runs BEFORE any I/O.
///
/// Rejects: empty, any `..` segment, and non-POSIX absolute shapes
/// (`C:\foo`, `C:/foo`, `\\server\share`).
fn validate_host_shape(prefix: &str, host: &str) -> Result<(), MountRegisterError> {
    if host.is_empty() {
        return Err(MountRegisterError::InvalidHost {
            prefix: prefix.to_string(),
            host: host.to_string(),
            reason: "host string is empty".to_string(),
        });
    }
    // Backslash rejection MUST precede components(): a string like
    // `foo\..\bar` parses as one Normal segment on POSIX but as
    // `[Normal, ParentDir, Normal]` on Windows, so the dotdot
    // detector would differ across platforms.
    if host.contains('\\') {
        return Err(MountRegisterError::InvalidHost {
            prefix: prefix.to_string(),
            host: host.to_string(),
            reason: "host must not contain '\\' (backslash); use POSIX-shape \
                     forward slashes only"
                .to_string(),
        });
    }
    if Path::new(host)
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(MountRegisterError::InvalidHost {
            prefix: prefix.to_string(),
            host: host.to_string(),
            reason: "host must not contain '..' segments".to_string(),
        });
    }
    if !host.starts_with('/') && looks_non_posix_absolute(host) {
        return Err(MountRegisterError::InvalidHost {
            prefix: prefix.to_string(),
            host: host.to_string(),
            reason: "host must use POSIX shape; rewrite with forward slashes \
                     and a leading '/' for an absolute path"
                .to_string(),
        });
    }
    Ok(())
}

/// Detect non-POSIX absolute path shapes (UNC, backslash root,
/// drive letter) that `resolve_against` would otherwise treat as
/// relative.
fn looks_non_posix_absolute(host: &str) -> bool {
    if host.starts_with('\\') {
        return true;
    }
    // Drive-letter form: alpha + `:` at offsets 0..=1.
    let bytes = host.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return true;
    }
    false
}

/// Reject `override_env = Some("")`.
fn validate_override_env_name(
    prefix: &str,
    env_name: Option<&str>,
) -> Result<(), MountRegisterError> {
    let Some(name) = env_name else {
        return Ok(());
    };
    if name.is_empty() {
        return Err(MountRegisterError::InvalidHost {
            prefix: prefix.to_string(),
            host: String::new(),
            reason: "override_env name is empty; either remove the field \
                     or set it to a real env-var name"
                .to_string(),
        });
    }
    Ok(())
}

/// Returns the env value when set non-empty after trimming; `None`
/// otherwise. Whitespace-only is treated as unset to avoid mounting
/// `workspace_root.join(" ")`.
fn override_host_from_env<F>(entry: &MountEntry, mut getter: F) -> Option<String>
where
    F: FnMut(&str) -> Option<String>,
{
    let env_name = entry.override_env.as_deref()?;
    let value = getter(env_name)?;
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

/// Probe `host_path` and return a typed error per failure shape.
fn check_host_root_is_dir(
    host_path: &Path,
    prefix: &str,
    override_env: &Option<String>,
) -> Result<(), MountRegisterError> {
    match std::fs::metadata(host_path) {
        Ok(md) if md.is_dir() => Ok(()),
        Ok(_) => Err(MountRegisterError::HostRootNotDirectory {
            prefix: prefix.to_string(),
            host_path: host_path.to_path_buf(),
            override_env: override_env.clone(),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(MountRegisterError::HostRootMissing {
                prefix: prefix.to_string(),
                host_path: host_path.to_path_buf(),
                override_env: override_env.clone(),
            })
        }
        Err(e) => Err(MountRegisterError::HostRootIo {
            prefix: prefix.to_string(),
            host_path: host_path.to_path_buf(),
            source: e,
            override_env: override_env.clone(),
        }),
    }
}

/// Register each manifest mount in `host.fs_mounts_mut()`. Relative
/// `host` paths resolve against `workspace_root`; `override_env`
/// (when set non-empty via `getter`) replaces the manifest's `host`.
///
/// # Per-entry validation order
///
/// 1. Prefix shape.
/// 2. `override_env` name shape.
/// 3. Manifest `host` shape.
/// 4. Env override resolution; resolved value re-validated for shape.
/// 5. Host directory probe.
/// 6. `std::fs::canonicalize`.
/// 7. `FsMount::new`, then in-slice and cross-call duplicate check
///    using the normalized prefix.
///
/// # Atomicity
///
/// All entries are validated and built into `FsMount` values before
/// any mutation of `host`. On any failure the mount table is untouched.
pub fn register_mounts<F>(
    mounts: &[MountEntry],
    workspace_root: &Path,
    mut getter: F,
    host: &mut Lv2Host,
) -> Result<usize, MountRegisterError>
where
    F: FnMut(&str) -> Option<String>,
{
    use std::collections::BTreeSet;

    // Snapshot existing prefixes so cross-call duplicates surface in
    // the validation phase.
    let existing_prefixes: BTreeSet<String> = host
        .fs_mounts()
        .mounts()
        .map(|m| m.prefix.clone())
        .collect();
    let mut seen_in_slice: BTreeSet<String> = BTreeSet::new();
    let mut prepared: Vec<FsMount> = Vec::with_capacity(mounts.len());

    for entry in mounts {
        validate_prefix(&entry.prefix)?;
        validate_override_env_name(&entry.prefix, entry.override_env.as_deref())?;
        validate_host_shape(&entry.prefix, &entry.host)?;

        let (host_string, override_env_for_err) = match override_host_from_env(entry, &mut getter) {
            Some(v) => (v, entry.override_env.clone()),
            None => (entry.host.clone(), None),
        };
        // Re-validate the env value: it obeys the same shape rules
        // as the committed manifest path.
        if override_env_for_err.is_some() {
            validate_host_shape(&entry.prefix, &host_string)?;
        }

        let host_path = resolve_against(workspace_root, &host_string);
        check_host_root_is_dir(&host_path, &entry.prefix, &override_env_for_err)?;
        let canonical = canonicalize_existing(&entry.prefix, &host_path, &override_env_for_err)?;

        // Prefix was just validated, so FsMount::new must succeed.
        // A None here means validate_prefix and FsMount::new have
        // diverged; surface as InvalidPrefix rather than panicking.
        let mount = FsMount::new(entry.prefix.clone(), canonical).ok_or_else(|| {
            MountRegisterError::InvalidPrefix {
                prefix: entry.prefix.clone(),
                reason: "FsMount::new rejected a prefix that passed validate_prefix; \
                         contract drift between the two validators"
                    .to_string(),
            }
        })?;

        // Use the FsMount-normalized prefix (no trailing slash) for
        // dedup so `/app_home` and `/app_home/` collide.
        let normalized = mount.prefix.clone();
        if existing_prefixes.contains(&normalized) || !seen_in_slice.insert(normalized.clone()) {
            return Err(MountRegisterError::DuplicatePrefix { prefix: normalized });
        }
        prepared.push(mount);
    }

    // Commit phase: full set or none.
    for mount in prepared {
        let prefix = mount.prefix.clone();
        host.fs_mounts_mut()
            .add(mount)
            .map_err(|_| MountRegisterError::DuplicatePrefix { prefix })?;
    }
    Ok(mounts.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TmpDir(PathBuf);

    impl TmpDir {
        fn new(name: &str) -> Self {
            let p =
                std::env::temp_dir().join(format!("cellgov_mounts_{name}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn entry(prefix: &str, host: &str, override_env: Option<&str>) -> MountEntry {
        MountEntry {
            prefix: prefix.to_string(),
            host: host.to_string(),
            override_env: override_env.map(|s| s.to_string()),
        }
    }

    /// Canonicalize on both sides so the `\\?\` prefix Windows adds
    /// does not break portable assertions.
    fn canon(p: &Path) -> PathBuf {
        std::fs::canonicalize(p).expect("canonicalize")
    }

    #[test]
    fn validate_host_shape_rejects_empty() {
        let err = validate_host_shape("/p", "").unwrap_err();
        assert!(matches!(err, MountRegisterError::InvalidHost { .. }));
    }

    #[test]
    fn validate_host_shape_rejects_dotdot_segment() {
        let err = validate_host_shape("/p", "assets/../escape").unwrap_err();
        match err {
            MountRegisterError::InvalidHost { reason, .. } => {
                assert!(reason.contains(".."), "reason names the rule");
            }
            other => panic!("expected InvalidHost, got {other}"),
        }
        assert!(validate_host_shape("/p", "../foo").is_err());
        assert!(validate_host_shape("/p", "/abs/../escape").is_err());
    }

    #[test]
    fn validate_host_shape_rejects_windows_drive_letter() {
        for shape in ["C:\\foo", "C:/foo", "C:", "Z:\\Users\\me"] {
            let err = validate_host_shape("/p", shape).expect_err(shape);
            assert!(
                matches!(err, MountRegisterError::InvalidHost { .. }),
                "{shape:?} -> {err:?}",
            );
        }
    }

    #[test]
    fn validate_host_shape_rejects_unc_and_backslash_root() {
        for shape in ["\\\\server\\share", "\\foo"] {
            let err = validate_host_shape("/p", shape).expect_err(shape);
            assert!(matches!(err, MountRegisterError::InvalidHost { .. }));
        }
    }

    #[test]
    fn validate_host_shape_rejects_mid_string_backslash() {
        let inputs = ["foo\\..\\bar", "assets\\sub", "a/b\\c"];
        for shape in inputs {
            let err = validate_host_shape("/p", shape).expect_err(shape);
            match err {
                MountRegisterError::InvalidHost { reason, .. } => {
                    assert!(
                        reason.contains("backslash"),
                        "{shape:?} reason should name backslash: {reason}",
                    );
                }
                other => panic!("{shape:?} -> {other:?}"),
            }
        }
    }

    #[test]
    fn validate_host_shape_accepts_posix_shapes() {
        validate_host_shape("/p", "tests/fixtures/foo").unwrap();
        validate_host_shape("/p", "/abs/path").unwrap();
        validate_host_shape("/p", "./relative").unwrap();
    }

    #[test]
    fn resolve_against_treats_leading_slash_as_absolute_on_every_platform() {
        let r = resolve_against(Path::new("/workspace"), "/abs");
        assert_eq!(r, PathBuf::from("/abs"));
    }

    #[test]
    fn resolve_against_joins_relative_paths_under_base() {
        let r = resolve_against(Path::new("/workspace"), "assets/sub");
        assert_eq!(r, PathBuf::from("/workspace").join("assets/sub"));
    }

    #[test]
    fn register_zero_entries_succeeds_with_count_zero() {
        let mut host = Lv2Host::new();
        let baseline = host.fs_mounts().mounts().count();
        let n = register_mounts(&[], Path::new("/unused"), |_| None, &mut host).unwrap();
        assert_eq!(n, 0);
        assert_eq!(host.fs_mounts().mounts().count(), baseline);
    }

    #[test]
    fn relative_host_resolves_under_workspace_and_canonicalizes() {
        let workspace = TmpDir::new("rel_host");
        std::fs::create_dir_all(workspace.path().join("assets")).unwrap();
        let mut host = Lv2Host::new();
        let entries = vec![entry("/app_home", "assets", None)];
        let n = register_mounts(&entries, workspace.path(), |_| None, &mut host).unwrap();
        assert_eq!(n, 1);
        let mount_host = host
            .fs_mounts()
            .mounts()
            .next()
            .expect("one mount")
            .host_root
            .clone();
        assert_eq!(mount_host, canon(&workspace.path().join("assets")));
    }

    #[test]
    fn missing_host_root_returns_typed_missing_error() {
        let workspace = TmpDir::new("missing_root");
        let mut host = Lv2Host::new();
        let entries = vec![entry("/app_home", "does/not/exist", None)];
        let err = register_mounts(&entries, workspace.path(), |_| None, &mut host)
            .expect_err("missing host root must surface");
        match err {
            MountRegisterError::HostRootMissing {
                prefix,
                override_env,
                ..
            } => {
                assert_eq!(prefix, "/app_home");
                assert!(override_env.is_none());
            }
            other => panic!("expected HostRootMissing, got {other}"),
        }
    }

    #[test]
    fn host_root_pointing_to_a_file_returns_not_directory_error() {
        let workspace = TmpDir::new("file_root");
        std::fs::write(workspace.path().join("not_a_dir"), b"x").unwrap();
        let mut host = Lv2Host::new();
        let entries = vec![entry("/app_home", "not_a_dir", None)];
        let err = register_mounts(&entries, workspace.path(), |_| None, &mut host)
            .expect_err("file-as-mount must surface");
        match err {
            MountRegisterError::HostRootNotDirectory { prefix, .. } => {
                assert_eq!(prefix, "/app_home");
            }
            other => panic!("expected HostRootNotDirectory, got {other}"),
        }
    }

    #[test]
    fn host_path_with_nul_byte_returns_host_root_io() {
        // NUL passes shape validation but metadata() rejects with
        // InvalidInput, exercising the HostRootIo arm portably.
        let workspace = TmpDir::new("nul_host");
        let mut host = Lv2Host::new();
        let entries = vec![entry("/app_home", "foo\0bar", None)];
        let err = register_mounts(&entries, workspace.path(), |_| None, &mut host)
            .expect_err("NUL-byte host must surface");
        match err {
            MountRegisterError::HostRootIo { prefix, source, .. } => {
                assert_eq!(prefix, "/app_home");
                assert!(!source.to_string().is_empty());
            }
            other => panic!("expected HostRootIo, got {other}"),
        }
    }

    #[test]
    fn override_env_replaces_manifest_host() {
        let workspace = TmpDir::new("override_workspace");
        std::fs::create_dir_all(workspace.path().join("manifest_dir")).unwrap();
        std::fs::create_dir_all(workspace.path().join("real_dir")).unwrap();
        let mut host = Lv2Host::new();
        let entries = vec![entry(
            "/app_home",
            "manifest_dir",
            Some("CELLGOV_TEST_OVERRIDE"),
        )];
        let getter = |name: &str| {
            if name == "CELLGOV_TEST_OVERRIDE" {
                Some("real_dir".to_string())
            } else {
                None
            }
        };
        register_mounts(&entries, workspace.path(), getter, &mut host).unwrap();
        let mount_host = host
            .fs_mounts()
            .mounts()
            .next()
            .expect("one mount")
            .host_root
            .clone();
        assert_eq!(mount_host, canon(&workspace.path().join("real_dir")));
    }

    #[test]
    fn empty_override_env_value_falls_through_to_manifest_host() {
        let workspace = TmpDir::new("empty_override");
        std::fs::create_dir_all(workspace.path().join("fallback")).unwrap();
        let mut host = Lv2Host::new();
        let entries = vec![entry(
            "/app_home",
            "fallback",
            Some("CELLGOV_EMPTY_OVERRIDE"),
        )];
        let getter = |name: &str| {
            if name == "CELLGOV_EMPTY_OVERRIDE" {
                Some(String::new())
            } else {
                None
            }
        };
        register_mounts(&entries, workspace.path(), getter, &mut host).unwrap();
        let mount_host = host
            .fs_mounts()
            .mounts()
            .next()
            .expect("one mount")
            .host_root
            .clone();
        assert_eq!(mount_host, canon(&workspace.path().join("fallback")));
    }

    #[test]
    fn whitespace_only_override_env_value_falls_through_to_manifest_host() {
        let workspace = TmpDir::new("ws_override");
        std::fs::create_dir_all(workspace.path().join("fallback")).unwrap();
        let mut host = Lv2Host::new();
        let entries = vec![entry("/app_home", "fallback", Some("CELLGOV_WS_OVERRIDE"))];
        let getter = |name: &str| {
            if name == "CELLGOV_WS_OVERRIDE" {
                Some(" \n\t ".to_string())
            } else {
                None
            }
        };
        register_mounts(&entries, workspace.path(), getter, &mut host).unwrap();
        let mount_host = host
            .fs_mounts()
            .mounts()
            .next()
            .expect("one mount")
            .host_root
            .clone();
        assert_eq!(mount_host, canon(&workspace.path().join("fallback")));
    }

    #[test]
    fn override_env_pointing_to_missing_dir_carries_env_name_in_error() {
        let workspace = TmpDir::new("env_missing_workspace");
        std::fs::create_dir_all(workspace.path().join("manifest_dir")).unwrap();
        let mut host = Lv2Host::new();
        let entries = vec![entry(
            "/app_home",
            "manifest_dir",
            Some("CELLGOV_MISSING_OVERRIDE"),
        )];
        let getter = |name: &str| {
            if name == "CELLGOV_MISSING_OVERRIDE" {
                Some("nope/does/not/exist".to_string())
            } else {
                None
            }
        };
        let err = register_mounts(&entries, workspace.path(), getter, &mut host)
            .expect_err("env-pointed missing dir must surface");
        match err {
            MountRegisterError::HostRootMissing { override_env, .. } => {
                assert_eq!(override_env.as_deref(), Some("CELLGOV_MISSING_OVERRIDE"));
            }
            other => panic!("expected HostRootMissing with env name, got {other}"),
        }
    }

    #[test]
    fn override_env_value_must_obey_host_shape_rules() {
        let workspace = TmpDir::new("env_shape");
        std::fs::create_dir_all(workspace.path().join("manifest_dir")).unwrap();
        let mut host = Lv2Host::new();
        let entries = vec![entry(
            "/app_home",
            "manifest_dir",
            Some("CELLGOV_BAD_OVERRIDE"),
        )];
        let getter = |name: &str| {
            if name == "CELLGOV_BAD_OVERRIDE" {
                Some("../escape".to_string())
            } else {
                None
            }
        };
        let err = register_mounts(&entries, workspace.path(), getter, &mut host)
            .expect_err("env-supplied dotdot must surface");
        assert!(matches!(err, MountRegisterError::InvalidHost { .. }));
    }

    #[test]
    fn empty_override_env_name_is_rejected() {
        let workspace = TmpDir::new("empty_env_name");
        std::fs::create_dir_all(workspace.path().join("manifest_dir")).unwrap();
        let mut host = Lv2Host::new();
        let entries = vec![entry("/app_home", "manifest_dir", Some(""))];
        let err = register_mounts(&entries, workspace.path(), |_| None, &mut host)
            .expect_err("empty env name must surface");
        match err {
            MountRegisterError::InvalidHost { reason, .. } => {
                assert!(reason.contains("override_env"), "reason names the field");
            }
            other => panic!("expected InvalidHost, got {other}"),
        }
    }

    #[test]
    fn empty_prefix_is_rejected_before_io() {
        let workspace = TmpDir::new("empty_prefix");
        let mut host = Lv2Host::new();
        let entries = vec![entry("", "missing/dir", None)];
        let err = register_mounts(&entries, workspace.path(), |_| None, &mut host)
            .expect_err("empty prefix must surface");
        assert!(matches!(err, MountRegisterError::InvalidPrefix { .. }));
    }

    #[test]
    fn unrooted_prefix_is_rejected_before_io() {
        let workspace = TmpDir::new("unrooted_prefix");
        let mut host = Lv2Host::new();
        let entries = vec![entry("app_home", "missing/dir", None)];
        let err = register_mounts(&entries, workspace.path(), |_| None, &mut host)
            .expect_err("non-rooted prefix must surface");
        match err {
            MountRegisterError::InvalidPrefix { prefix, reason } => {
                assert_eq!(prefix, "app_home");
                assert!(reason.contains("/"), "reason should mention rooting");
            }
            other => panic!("expected InvalidPrefix, got {other}"),
        }
    }

    #[test]
    fn dotdot_in_prefix_is_rejected_before_io() {
        let workspace = TmpDir::new("dotdot_prefix");
        std::fs::create_dir_all(workspace.path().join("ok_dir")).unwrap();
        let mut host = Lv2Host::new();
        let entries = vec![entry("/app_home/../etc", "ok_dir", None)];
        let err = register_mounts(&entries, workspace.path(), |_| None, &mut host)
            .expect_err("invalid prefix must surface");
        assert!(matches!(err, MountRegisterError::InvalidPrefix { .. }));
    }

    #[test]
    fn empty_host_string_is_rejected_before_io() {
        let workspace = TmpDir::new("empty_host_workspace");
        let mut host = Lv2Host::new();
        let entries = vec![entry("/app_home", "", None)];
        let err = register_mounts(&entries, workspace.path(), |_| None, &mut host)
            .expect_err("empty host must surface");
        match err {
            MountRegisterError::InvalidHost { prefix, host, .. } => {
                assert_eq!(prefix, "/app_home");
                assert_eq!(host, "");
            }
            other => panic!("expected InvalidHost, got {other}"),
        }
    }

    #[test]
    fn dotdot_in_host_is_rejected() {
        let workspace = TmpDir::new("dotdot_host");
        let mut host = Lv2Host::new();
        let entries = vec![entry("/app_home", "../escape", None)];
        let err = register_mounts(&entries, workspace.path(), |_| None, &mut host)
            .expect_err("dotdot host must surface");
        match err {
            MountRegisterError::InvalidHost { reason, .. } => {
                assert!(reason.contains(".."), "reason names the rule");
            }
            other => panic!("expected InvalidHost, got {other}"),
        }
    }

    #[test]
    fn windows_shape_host_is_rejected_for_determinism() {
        let workspace = TmpDir::new("win_host");
        let mut host = Lv2Host::new();
        let entries = vec![entry("/app_home", "C:\\Users\\me\\flow", None)];
        let err = register_mounts(&entries, workspace.path(), |_| None, &mut host)
            .expect_err("windows-shape host must surface");
        assert!(matches!(err, MountRegisterError::InvalidHost { .. }));
    }

    #[test]
    fn invalid_prefix_takes_precedence_over_missing_host() {
        let workspace = TmpDir::new("precedence");
        let mut host = Lv2Host::new();
        let entries = vec![entry("not_rooted", "path/does/not/exist", None)];
        let err = register_mounts(&entries, workspace.path(), |_| None, &mut host)
            .expect_err("must surface");
        assert!(
            matches!(err, MountRegisterError::InvalidPrefix { .. }),
            "expected InvalidPrefix, got {err:?}",
        );
    }

    #[test]
    fn registration_order_matches_manifest_order() {
        let workspace = TmpDir::new("order");
        std::fs::create_dir_all(workspace.path().join("app_home")).unwrap();
        std::fs::create_dir_all(workspace.path().join("dev_hdd0")).unwrap();
        let mut host = Lv2Host::new();
        let entries = vec![
            entry("/dev_hdd0", "dev_hdd0", None),
            entry("/app_home", "app_home", None),
        ];
        register_mounts(&entries, workspace.path(), |_| None, &mut host).unwrap();
        let prefixes: Vec<&str> = host
            .fs_mounts()
            .mounts()
            .map(|m| m.prefix.as_str())
            .collect();
        assert_eq!(prefixes, vec!["/dev_hdd0", "/app_home"]);
    }

    #[test]
    fn first_failure_leaves_host_mount_table_untouched() {
        let workspace = TmpDir::new("atomicity");
        std::fs::create_dir_all(workspace.path().join("ok_dir")).unwrap();
        let mut host = Lv2Host::new();
        let baseline = host.fs_mounts().mounts().count();
        let entries = vec![
            entry("/ok", "ok_dir", None),
            entry("/missing", "does_not_exist", None),
        ];
        let _err = register_mounts(&entries, workspace.path(), |_| None, &mut host)
            .expect_err("entry 1 must fail");
        assert_eq!(
            host.fs_mounts().mounts().count(),
            baseline,
            "host mount table must be untouched after first-failure",
        );
    }

    #[test]
    fn in_slice_duplicate_prefix_is_rejected() {
        let workspace = TmpDir::new("slice_dup");
        std::fs::create_dir_all(workspace.path().join("a")).unwrap();
        std::fs::create_dir_all(workspace.path().join("b")).unwrap();
        let mut host = Lv2Host::new();
        let entries = vec![entry("/app_home", "a", None), entry("/app_home", "b", None)];
        let err = register_mounts(&entries, workspace.path(), |_| None, &mut host)
            .expect_err("in-slice duplicate must surface");
        assert!(matches!(err, MountRegisterError::DuplicatePrefix { .. }));
        assert_eq!(host.fs_mounts().mounts().count(), 0);
    }

    #[test]
    fn trailing_slash_prefix_dedup_matches_fsmount_normalization() {
        // FsMount::new strips trailing `/`, so `/app_home` and
        // `/app_home/` collide after normalization.
        let workspace = TmpDir::new("trailing_slash");
        std::fs::create_dir_all(workspace.path().join("a")).unwrap();
        std::fs::create_dir_all(workspace.path().join("b")).unwrap();
        let mut host = Lv2Host::new();
        let entries = vec![
            entry("/app_home", "a", None),
            entry("/app_home/", "b", None),
        ];
        let err = register_mounts(&entries, workspace.path(), |_| None, &mut host)
            .expect_err("trailing-slash collision must surface");
        assert!(matches!(err, MountRegisterError::DuplicatePrefix { .. }));
    }

    #[test]
    fn cross_call_duplicate_prefix_is_rejected() {
        let workspace = TmpDir::new("cross_call");
        std::fs::create_dir_all(workspace.path().join("a")).unwrap();
        std::fs::create_dir_all(workspace.path().join("b")).unwrap();
        let mut host = Lv2Host::new();
        register_mounts(
            &[entry("/app_home", "a", None)],
            workspace.path(),
            |_| None,
            &mut host,
        )
        .unwrap();
        let err = register_mounts(
            &[entry("/app_home", "b", None)],
            workspace.path(),
            |_| None,
            &mut host,
        )
        .expect_err("cross-call duplicate must surface");
        assert!(matches!(err, MountRegisterError::DuplicatePrefix { .. }));
        assert_eq!(host.fs_mounts().mounts().count(), 1);
    }

    #[test]
    fn disjoint_register_mounts_calls_compose() {
        let workspace = TmpDir::new("disjoint");
        std::fs::create_dir_all(workspace.path().join("a")).unwrap();
        std::fs::create_dir_all(workspace.path().join("b")).unwrap();
        let mut host = Lv2Host::new();
        register_mounts(
            &[entry("/app_home", "a", None)],
            workspace.path(),
            |_| None,
            &mut host,
        )
        .unwrap();
        register_mounts(
            &[entry("/dev_hdd0", "b", None)],
            workspace.path(),
            |_| None,
            &mut host,
        )
        .unwrap();
        assert_eq!(host.fs_mounts().mounts().count(), 2);
    }
}
