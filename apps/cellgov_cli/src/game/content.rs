//! Boot-time content provider.
//!
//! Reads each [`ContentManifest`] entry off the host filesystem and
//! registers the bytes in [`Lv2Host::fs_store_mut`] under the manifest's
//! `guest_path`. A missing host file is a startup error rather than a
//! silent ENOENT-at-runtime.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cellgov_lv2::{FsError, Lv2Host};

use super::manifest::ContentManifest;

/// Why [`register_content_blobs`] could not register a manifest's
/// content.
#[derive(Debug, thiserror::Error)]
pub enum ContentRegisterError {
    /// Reading the host file failed (NotFound, permission-denied, IO).
    #[error(
        "content: failed to read host file {} for guest path {:?}: {source}{}",
        host_path.display(),
        guest_path,
        render_override_hint(override_env)
    )]
    HostFileRead {
        guest_path: String,
        host_path: PathBuf,
        #[source]
        source: std::io::Error,
        /// Override env var name, surfaced in Display so a developer
        /// sees which env they need to fix.
        override_env: Option<String>,
    },
    /// Two manifest entries name the same `guest_path`.
    #[error(
        "content: duplicate guest path {:?} in manifest (first host source {}, second host source {})",
        guest_path,
        first_host_path.display(),
        second_host_path.display(),
    )]
    DuplicateGuestPath {
        guest_path: String,
        first_host_path: PathBuf,
        second_host_path: PathBuf,
    },
    /// A manifest entry names a `guest_path` the host registers itself
    /// before any manifest is read.
    #[error(
        "content: guest path {:?} (host source {}) is one the LV2 host registers itself; a \
         manifest cannot supply it",
        guest_path,
        host_path.display(),
    )]
    GuestPathBuiltIn {
        guest_path: String,
        host_path: PathBuf,
    },
    /// No base directory: the override env var is unset or empty, and
    /// the caller gave no EBOOT directory.
    #[error(
        "content: no base directory for {n} manifest entr{}: {} and the EBOOT path has no \
         parent directory",
        if *n == 1 { "y" } else { "ies" },
        render_no_override(override_env)
    )]
    NoBase {
        n: usize,
        override_env: Option<String>,
    },
}

fn render_override_hint(override_env: &Option<String>) -> String {
    match override_env {
        Some(env) => format!(
            " (override env var {env} is set; either drop the \
             file into that directory or unset {env} to read \
             from the EBOOT's own directory)"
        ),
        None => String::new(),
    }
}

/// The hint follows the rule of [`override_base_from_env`]: an empty
/// value counts as unset.
fn render_no_override(override_env: &Option<String>) -> String {
    match override_env {
        Some(env) => format!("{env} is unset or empty"),
        None => "the manifest declares no override_base_env".to_string(),
    }
}

/// Resolve `path` against `base` if relative; absolute passes through.
/// Pure path arithmetic, no I/O.
fn resolve(base: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    }
}

/// Source of the resolved content base directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentBaseSource {
    /// The directory the EBOOT sits in, taken as given (no probe).
    Usrdir { path: PathBuf },
    /// Override env var named by `[content] override_base_env`.
    Override { env: String },
}

/// Read each manifest entry off disk and register the bytes in
/// `host.fs_store_mut`.
///
/// The base directory is the first of these that is `Some`:
///
/// 1. `override_base`, joined onto `workspace_root` when relative
/// 2. `usrdir_base`, taken as given
///
/// A relative `host_path` resolves against that base.
///
/// # Errors
///
/// - [`ContentRegisterError::NoBase`]: both bases are `None`.
/// - [`ContentRegisterError::HostFileRead`]: a file is missing under
///   the chosen base; the error names the path it probed.
/// - [`ContentRegisterError::DuplicateGuestPath`]: an earlier entry of
///   this manifest registered the same `guest_path`.
/// - [`ContentRegisterError::GuestPathBuiltIn`]: the host registered
///   the `guest_path` itself, before this manifest was read.
///
/// Registration stops at the first failure; the `FsStore` keeps
/// whatever earlier entries registered.
pub fn register_content_blobs(
    manifest: &ContentManifest,
    workspace_root: &Path,
    override_base: Option<&Path>,
    usrdir_base: Option<&Path>,
    host: &mut Lv2Host,
) -> Result<ContentBaseSource, ContentRegisterError> {
    let (base, source) = match (override_base, usrdir_base) {
        // `Path::join` keeps a rooted `p` as is, so a relative override
        // resolves against `workspace_root` and an absolute one passes
        // through.
        (Some(p), _) => (
            workspace_root.join(p),
            ContentBaseSource::Override {
                env: manifest
                    .override_base_env
                    .clone()
                    .unwrap_or_else(|| "<no override_base_env declared>".to_string()),
            },
        ),
        (None, Some(usrdir)) => (
            usrdir.to_path_buf(),
            ContentBaseSource::Usrdir {
                path: usrdir.to_path_buf(),
            },
        ),
        (None, None) => {
            return Err(ContentRegisterError::NoBase {
                n: manifest.files.len(),
                override_env: manifest.override_base_env.clone(),
            });
        }
    };
    let override_env_for_err = match &source {
        ContentBaseSource::Override { env } => Some(env.clone()),
        ContentBaseSource::Usrdir { .. } => None,
    };
    // The host source each guest path of this manifest registered
    // from, so a collision can say whether the earlier registration
    // was the manifest's own or the host's built-in set.
    let mut registered: BTreeMap<&str, PathBuf> = BTreeMap::new();
    for entry in &manifest.files {
        let host_path = resolve(&base, &entry.host_path);
        let bytes =
            std::fs::read(&host_path).map_err(|io_err| ContentRegisterError::HostFileRead {
                guest_path: entry.guest_path.clone(),
                host_path: host_path.clone(),
                source: io_err,
                override_env: override_env_for_err.clone(),
            })?;
        if let Err(FsError::PathAlreadyRegistered) = host
            .fs_store_mut()
            .register_blob(entry.guest_path.clone(), bytes)
        {
            return Err(match registered.get(entry.guest_path.as_str()) {
                Some(prior) => ContentRegisterError::DuplicateGuestPath {
                    guest_path: entry.guest_path.clone(),
                    first_host_path: prior.clone(),
                    second_host_path: host_path,
                },
                None => ContentRegisterError::GuestPathBuiltIn {
                    guest_path: entry.guest_path.clone(),
                    host_path,
                },
            });
        }
        registered.insert(entry.guest_path.as_str(), host_path);
    }
    Ok(source)
}

/// Look up the override base directory selected by a manifest's
/// `override_base_env`.
///
/// Returns `Some(path)` when the env var holds more than whitespace,
/// else `None`. The mount provider applies the same rule to a var a
/// manifest shares between the two.
///
/// Takes a `getter` so tests can run without mutating process env.
pub fn override_base_from_env<F>(manifest: &ContentManifest, mut getter: F) -> Option<PathBuf>
where
    F: FnMut(&str) -> Option<String>,
{
    let env_name = manifest.override_base_env.as_deref()?;
    let value = getter(env_name)?;
    if value.trim().is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

#[cfg(test)]
#[path = "tests/content_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/content_collision_tests.rs"]
mod collision_tests;
