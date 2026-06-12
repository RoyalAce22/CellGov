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
#[derive(Debug, thiserror::Error)]
pub enum MountRegisterError {
    /// Prefix failed shape validation (empty, non-`/`, `..` segment).
    #[error("mounts: prefix {prefix:?} is invalid: {reason}")]
    InvalidPrefix { prefix: String, reason: String },
    /// Host string failed shape validation (empty, `..` segment,
    /// non-POSIX absolute shape, or empty `override_env` name).
    #[error("mounts: host {host:?} for prefix {prefix:?} is invalid: {reason}")]
    InvalidHost {
        prefix: String,
        host: String,
        reason: String,
    },
    /// Host root does not exist.
    #[error(
        "mounts: prefix {:?} resolved to {} which does not exist{}",
        prefix,
        host_path.display(),
        render_override_hint(override_env)
    )]
    HostRootMissing {
        prefix: String,
        host_path: PathBuf,
        /// Override env var name, surfaced in Display.
        override_env: Option<String>,
    },
    /// Host root exists but is not a directory.
    #[error(
        "mounts: prefix {:?} resolved to {} which exists but is not a directory{}",
        prefix,
        host_path.display(),
        render_override_hint(override_env)
    )]
    HostRootNotDirectory {
        prefix: String,
        host_path: PathBuf,
        override_env: Option<String>,
    },
    /// Host-root metadata probe failed (non-NotFound I/O error).
    #[error(
        "mounts: prefix {:?} could not stat {}: {source}{}",
        prefix,
        host_path.display(),
        render_override_hint(override_env)
    )]
    HostRootIo {
        prefix: String,
        host_path: PathBuf,
        #[source]
        source: std::io::Error,
        override_env: Option<String>,
    },
    /// `FsMountTable::add` rejected the prefix.
    #[error("mounts: prefix {prefix:?} is already registered (FsMountTable rejected it)")]
    DuplicatePrefix { prefix: String },
}

fn render_override_hint(override_env: &Option<String>) -> String {
    match override_env {
        Some(env) => format!(
            " (override env var {env} is set; either point it at a real directory or \
             unset {env} to fall back to the manifest's checked-in host)"
        ),
        None => String::new(),
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
#[path = "tests/mounts_tests.rs"]
mod tests;
