//! Boot-time mount-table provider.
//!
//! Reads each entry of a title's `[[fs.mounts]]` block, resolves the
//! host directory (with optional per-mount env-var override), and
//! adds a [`cellgov_lv2::FsMount`] to [`Lv2Host::fs_mounts_mut`].
//! Called once during `super::boot::prepare`, before any
//! `sys_fs_open` could land in the dispatch layer.
//!
//! A missing or non-directory host root is a startup error rather
//! than a silent ENOENT-at-runtime: a misconfigured manifest must
//! not look like a runtime FS bug. Mirrors `super::content`'s
//! "fail loud at startup" policy.
//!
//! # Validation order
//!
//! Per-entry the helper runs all pure-shape validation (prefix and
//! host string) BEFORE any I/O or env lookup. This way a manifest
//! with two co-occurring problems (e.g. an invalid prefix and a
//! missing host directory) surfaces the prefix error first; the
//! developer fixes the prefix, re-runs, and gets the host error
//! next, instead of fixing the host only to hit the prefix error
//! on the next attempt.

use std::path::{Path, PathBuf};

use cellgov_lv2::{FsMount, Lv2Host};

use super::manifest::MountEntry;

/// Why [`register_mounts`] could not register a mount entry.
#[derive(Debug)]
pub enum MountRegisterError {
    /// `[[fs.mounts]] prefix = ...` failed shape validation. The
    /// manifest loader catches the empty / non-`/` shapes earlier;
    /// this helper guards against an upstream change that bypasses
    /// the loader (e.g. a future `--mount` CLI flag) and also covers
    /// the `..`-segment case the loader does not currently check.
    InvalidPrefix { prefix: String, reason: String },
    /// `[[fs.mounts]] host = ...` (or the resolved `override_env`
    /// value) failed shape validation. Surfaces:
    /// - empty host string,
    /// - `..` segment anywhere in the host path,
    /// - non-POSIX absolute shapes (Windows drive letter, UNC,
    ///   backslash root) that would resolve differently across
    ///   platforms,
    /// - empty `override_env` name.
    InvalidHost {
        prefix: String,
        host: String,
        reason: String,
    },
    /// Host root does not exist (`std::io::ErrorKind::NotFound`).
    HostRootMissing {
        prefix: String,
        host_path: PathBuf,
        /// Name of the override env var that selected the host
        /// path, when applicable. Surfaced in Display so a developer
        /// who forgot to point the env at a real directory sees the
        /// var name in the error.
        override_env: Option<String>,
    },
    /// Host root exists but is not a directory (regular file,
    /// symlink-to-file, special, etc.).
    HostRootNotDirectory {
        prefix: String,
        host_path: PathBuf,
        override_env: Option<String>,
    },
    /// Host-root metadata probe failed for a reason other than
    /// NotFound (permission denied, IO error, broken symlink with
    /// a non-NotFound errno, etc.). Carrying the source error lets
    /// the developer distinguish "wrong path" from "permission bug
    /// on the right path."
    HostRootIo {
        prefix: String,
        host_path: PathBuf,
        source: std::io::Error,
        override_env: Option<String>,
    },
    /// `FsMountTable::add` rejected the prefix (already registered).
    /// The manifest loader catches duplicates earlier; this guards
    /// against an upstream change that bypasses validation.
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

/// Resolve `path` against `base` using POSIX-shape rules: a
/// leading `/` makes it absolute on every host; otherwise relative
/// to `base`. Pure path arithmetic, no I/O.
///
/// `Path::is_absolute` is deliberately NOT used here -- on Windows
/// a string like `/abs/path` is "drive-relative," not absolute, so
/// `Path::is_absolute` returns `false` and `base.join("/abs/path")`
/// silently produces a different path than on Linux. The state-hash
/// contract folds blob content into the observation; cross-platform
/// path divergence becomes a byte-identical-replay break.
///
/// `validate_host_shape` rejects Windows-shape inputs (`C:\foo`,
/// `\\server\share`) so non-POSIX strings never reach this resolver.
fn resolve_against(base: &Path, path: &str) -> PathBuf {
    if path.starts_with('/') {
        PathBuf::from(path)
    } else {
        base.join(path)
    }
}

/// Canonicalize a directory path that has already been verified to
/// exist via `check_host_root_is_dir`. Surfacing the canonical path
/// makes the mount table robust against symlinks anywhere in the
/// chain: two checkouts where one path passes through a symlink
/// and another does not produce the same `host_root` after this
/// pass, so the FsStore blob cache and any downstream trace records
/// are byte-identical.
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

/// Pure-shape prefix validation. Called BEFORE any I/O or env
/// lookup so a multi-error manifest surfaces shape problems first.
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

/// Pure-shape host-string validation. Called BEFORE any I/O so a
/// shape error always wins over an I/O error.
///
/// Rejects:
/// - empty (would silently map to workspace_root via
///   `Path::join("")`).
/// - any `..` segment (a manifest must not be able to read files
///   above its declared host root via path arithmetic).
/// - non-POSIX absolute shapes (`C:\foo`, `C:/foo`,
///   `\\server\share`). The resolver treats these as relative on
///   non-Windows hosts, which would join the path under the
///   workspace silently. POSIX-shape determinism wins on every
///   platform.
fn validate_host_shape(prefix: &str, host: &str) -> Result<(), MountRegisterError> {
    if host.is_empty() {
        return Err(MountRegisterError::InvalidHost {
            prefix: prefix.to_string(),
            host: host.to_string(),
            reason: "host string is empty".to_string(),
        });
    }
    // Backslash byte rejection MUST run before the components()
    // scan: `Path::new("foo\\..\\bar").components()` parses as a
    // single Normal segment on POSIX hosts (no ParentDir found,
    // would pass the dotdot check) but as `[Normal, ParentDir,
    // Normal]` on Windows. Pre-rejecting backslash closes that
    // platform-dependent parsing window. PSL1GHT / PS3 paths are
    // POSIX-only so any `\` in a manifest host is wrong.
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

/// Heuristic detector for non-POSIX absolute path shapes that
/// would otherwise be treated as "relative" by `resolve_against`
/// and silently joined under `workspace_root`.
fn looks_non_posix_absolute(host: &str) -> bool {
    if host.starts_with('\\') {
        // UNC (`\\server\share`) and backslash-rooted absolutes
        // (`\foo`).
        return true;
    }
    // Drive-letter form: alpha + `:` at offsets 0..=1, regardless
    // of whether the third byte is `/` or `\` or absent.
    let bytes = host.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return true;
    }
    false
}

/// Reject `override_env = Some("")` -- an empty env-var name is a
/// manifest config error, not a runtime fall-through. Surfaced as
/// `InvalidHost` reusing the field for the offending env name; the
/// `reason` makes the source of the error unambiguous.
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

/// Look up `entry`'s `override_env` via `getter`. Returns the env
/// value when set non-empty (after trimming) and the env-var name
/// is valid; `None` otherwise (empty / whitespace-only value,
/// absent var, or `override_env` unset). Whitespace-only values are
/// treated as absent: a developer who exported `MY_OVERRIDE=" "`
/// almost certainly meant "unset," and the alternative is to mount
/// `workspace_root.join(" ")` which fails opaquely as
/// `HostRootMissing`.
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

/// Probe `host_path` and return a typed error variant for each
/// distinct failure shape (NotFound, exists-but-not-a-directory,
/// other I/O). Caller fills in `prefix` / `override_env` for the
/// surfaced error.
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
/// `host` paths are resolved against `workspace_root`; per-mount
/// `override_env` (when set non-empty in the process env via
/// `getter`) replaces the manifest's `host`.
///
/// # Per-entry validation order
///
/// 1. Prefix shape (no I/O, no env).
/// 2. `override_env` name shape (no I/O, no env).
/// 3. Manifest `host` shape (no I/O, no env).
/// 4. Env override resolution; resolved value re-validated for
///    shape (env-supplied paths obey the same rules as manifest
///    paths).
/// 5. Host directory probe (typed error per failure mode).
/// 6. `std::fs::canonicalize` to pin the symlink-resolved path.
/// 7. `FsMount::new` (normalizes the prefix), then in-slice and
///    cross-call duplicate check using the normalized prefix.
///
/// # Atomicity
///
/// All entries are validated and built into ready `FsMount` values
/// before any mutation of `host`. If any entry fails the host's
/// mount table is left untouched. The commit phase that follows
/// can only fail on contract drift between `FsMountTable::add` and
/// our snapshot -- not on guest- or manifest-induced inputs.
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

    // Snapshot existing prefixes so cross-call duplicates surface
    // during validation, not in the commit phase.
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
        // Re-validate when the override resolved to a different
        // string: a developer's local env value must obey the same
        // shape rules as the committed manifest path.
        if override_env_for_err.is_some() {
            validate_host_shape(&entry.prefix, &host_string)?;
        }

        let host_path = resolve_against(workspace_root, &host_string);
        check_host_root_is_dir(&host_path, &entry.prefix, &override_env_for_err)?;
        let canonical = canonicalize_existing(&entry.prefix, &host_path, &override_env_for_err)?;

        // Prefix was just validated, so `FsMount::new` must succeed.
        // If it does not, validate_prefix and FsMount::new have
        // diverged -- a contract drift, surfaced as InvalidPrefix
        // rather than a panic.
        let mount = FsMount::new(entry.prefix.clone(), canonical).ok_or_else(|| {
            MountRegisterError::InvalidPrefix {
                prefix: entry.prefix.clone(),
                reason: "FsMount::new rejected a prefix that passed validate_prefix; \
                         contract drift between the two validators"
                    .to_string(),
            }
        })?;

        // Use the FsMount-normalized prefix (no trailing slash) for
        // dedup so `/app_home` and `/app_home/` collide consistently
        // with FsMountTable's matching rules.
        let normalized = mount.prefix.clone();
        if existing_prefixes.contains(&normalized) || !seen_in_slice.insert(normalized.clone()) {
            return Err(MountRegisterError::DuplicatePrefix { prefix: normalized });
        }
        prepared.push(mount);
    }

    // Commit phase: every entry validated, host gets the full set
    // or none. add() can only fail here on contract drift.
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

    /// RAII tempdir for register_mounts tests.
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

    /// `std::fs::canonicalize` adds a `\\?\` extended-length prefix
    /// on Windows. Mount-table comparisons must use the same
    /// canonical form on both sides so the assertion is portable.
    fn canon(p: &Path) -> PathBuf {
        std::fs::canonicalize(p).expect("canonicalize")
    }

    // -- pure-shape unit tests --

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
        // Pure leading `..` also rejected.
        assert!(validate_host_shape("/p", "../foo").is_err());
        // `..` in absolute path also rejected (defensive: traversal
        // semantics across symlinks are surprising).
        assert!(validate_host_shape("/p", "/abs/../escape").is_err());
    }

    #[test]
    fn validate_host_shape_rejects_windows_drive_letter() {
        // Without this rule, `C:\foo` resolves to `<workspace>/C:\foo`
        // on Linux silently. Forcing POSIX shape surfaces the
        // mistake at startup.
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
        // Pin: a mid-string backslash like `foo\..\bar` parses as
        // a single Normal component on POSIX and as
        // [Normal, ParentDir, Normal] on Windows. The dotdot
        // detector therefore yields different verdicts across
        // platforms. The byte-level backslash rejection runs
        // before components() to close that window.
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
        // Both forms are deterministic across platforms.
        validate_host_shape("/p", "tests/fixtures/foo").unwrap();
        validate_host_shape("/p", "/abs/path").unwrap();
        validate_host_shape("/p", "./relative").unwrap();
    }

    #[test]
    fn resolve_against_treats_leading_slash_as_absolute_on_every_platform() {
        // `Path::is_absolute` would return false here on Windows;
        // the POSIX-shape rule keeps behavior identical across hosts.
        let r = resolve_against(Path::new("/workspace"), "/abs");
        assert_eq!(r, PathBuf::from("/abs"));
    }

    #[test]
    fn resolve_against_joins_relative_paths_under_base() {
        let r = resolve_against(Path::new("/workspace"), "assets/sub");
        assert_eq!(r, PathBuf::from("/workspace").join("assets/sub"));
    }

    // -- I/O-bearing tests (use a workspace tempdir + relative hosts) --

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
        // Compare canonicalized form on both sides so the test
        // works whether or not the workspace itself contains
        // symlinks.
        assert_eq!(mount_host, canon(&workspace.path().join("assets")));
    }

    #[test]
    fn missing_host_root_returns_typed_missing_error() {
        let workspace = TmpDir::new("missing_root");
        let mut host = Lv2Host::new();
        // Workspace-relative path that doesn't exist; portable
        // across hosts (avoids Windows-shape paths).
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
        // Coverage for the HostRootIo arm. A NUL byte in the host
        // string passes validate_host_shape (it has no `\`, no
        // `..`, no leading drive letter, and is non-empty), then
        // metadata() rejects it with InvalidInput -- a non-NotFound
        // error that maps to HostRootIo with the source attached.
        // Cheap, portable substitute for a permission-denied probe
        // (which is awkward to synthesize cross-platform).
        let workspace = TmpDir::new("nul_host");
        let mut host = Lv2Host::new();
        let entries = vec![entry("/app_home", "foo\0bar", None)];
        let err = register_mounts(&entries, workspace.path(), |_| None, &mut host)
            .expect_err("NUL-byte host must surface");
        match err {
            MountRegisterError::HostRootIo { prefix, source, .. } => {
                assert_eq!(prefix, "/app_home");
                // Source attached so the developer can distinguish
                // "wrong path" from "permission bug" at log read.
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
        // Pin the trim-then-empty-fallthrough rule: a developer
        // who exported `MY_OVERRIDE=" "` almost certainly meant
        // "unset," and silently joining `workspace_root.join(" ")`
        // would fail opaquely as HostRootMissing.
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
        // A developer's local env var still has to be POSIX-shape.
        // Without this check, `MY_OVERRIDE="../../escape"` would
        // resolve under workspace_root and read sibling content.
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
        // `override_env = Some("")` is a manifest config error; an
        // empty env-var name can never be set, so the field is
        // useless / probably a typo.
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
        // Empty prefix + missing host: shape error wins because
        // shape validation runs before the I/O probe.
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
        // Manifest loader rejects this earlier in normal flow; the
        // helper guards against a future bypass.
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
        // Without this check, `Path::join("")` returns `base` and the
        // metadata probe succeeds (the workspace root almost always
        // exists), silently mounting the workspace as the title's
        // host root.
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
        // A title manifest must not be able to read sibling files
        // via `host = "../../etc"`; resolution would land outside
        // workspace_root.
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
        // Pin the cross-platform divergence fix: a manifest with a
        // Windows-shape host on Linux must NOT silently join the
        // path under the workspace.
        let workspace = TmpDir::new("win_host");
        let mut host = Lv2Host::new();
        let entries = vec![entry("/app_home", "C:\\Users\\me\\flow", None)];
        let err = register_mounts(&entries, workspace.path(), |_| None, &mut host)
            .expect_err("windows-shape host must surface");
        assert!(matches!(err, MountRegisterError::InvalidHost { .. }));
    }

    #[test]
    fn invalid_prefix_takes_precedence_over_missing_host() {
        // Co-occurring shape error + I/O error: shape wins so the
        // developer sees the prefix problem first and fixes it
        // before they hit the host problem.
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
        // Pin the pre-validate-then-commit atomicity contract:
        // entry 0 is valid, entry 1 fails. The host's mount table
        // must not have entry 0 partially registered.
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
        // Two entries with the same normalized prefix in the same
        // slice. Manifest loader catches this earlier; the helper
        // guards against bypass.
        let workspace = TmpDir::new("slice_dup");
        std::fs::create_dir_all(workspace.path().join("a")).unwrap();
        std::fs::create_dir_all(workspace.path().join("b")).unwrap();
        let mut host = Lv2Host::new();
        let entries = vec![entry("/app_home", "a", None), entry("/app_home", "b", None)];
        let err = register_mounts(&entries, workspace.path(), |_| None, &mut host)
            .expect_err("in-slice duplicate must surface");
        assert!(matches!(err, MountRegisterError::DuplicatePrefix { .. }));
        // Atomicity: entry 0 must not have been committed.
        assert_eq!(host.fs_mounts().mounts().count(), 0);
    }

    #[test]
    fn trailing_slash_prefix_dedup_matches_fsmount_normalization() {
        // FsMount::new strips trailing `/`. Two entries
        // [`/app_home`, `/app_home/`] therefore normalize to the
        // same prefix and must be rejected as duplicates -- a
        // future change to FsMount's normalization rules would
        // surface here.
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
        // Calling register_mounts twice on the same host with a
        // colliding prefix must reject the second call as a
        // duplicate, with no commit-phase mutation.
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
        // Mount table should still have exactly the first entry.
        assert_eq!(host.fs_mounts().mounts().count(), 1);
    }

    #[test]
    fn disjoint_register_mounts_calls_compose() {
        // Two disjoint calls compose: each registers its entries
        // independently. Pinning so future tightening doesn't
        // break the additive shape.
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
