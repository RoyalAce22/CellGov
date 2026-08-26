//! Boot-time content provider.
//!
//! Reads each [`ContentManifest`] entry off the host filesystem and
//! registers the bytes in [`Lv2Host::fs_store_mut`] under the manifest's
//! `guest_path`. A missing host file is a startup error rather than a
//! silent ENOENT-at-runtime.

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
}

fn render_override_hint(override_env: &Option<String>) -> String {
    match override_env {
        Some(env) => format!(
            " (override env var {env} is set; either drop the \
             file into that directory or unset {env} to fall \
             back to the manifest's checked-in base)"
        ),
        None => String::new(),
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
    /// Manifest's checked-in `base`.
    Manifest,
    /// EBOOT-relative USRDIR auto-discovered.
    Usrdir { path: PathBuf },
    /// Override env var named by `[content] override_base_env`.
    Override { env: String },
}

/// Soft probe for the USRDIR tier: a partial USRDIR falls through to
/// the manifest's checked-in base.
fn all_entries_resolve_under(base: &Path, manifest: &ContentManifest) -> bool {
    manifest.files.iter().all(|entry| {
        let p = resolve(base, &entry.host_path);
        std::fs::metadata(&p).map(|m| m.is_file()).unwrap_or(false)
    })
}

/// Read each manifest entry off disk and register the bytes in
/// `host.fs_store_mut`.
///
/// Base directory, first match wins: `override_base`, then
/// `usrdir_base` when every manifest entry resolves under it, then
/// `manifest.base`. A relative base resolves against
/// `workspace_root`; a relative `host_path` resolves against the
/// chosen base.
///
/// # Errors
///
/// Stops at the first failure; the `FsStore` keeps whatever earlier
/// entries registered.
pub fn register_content_blobs(
    manifest: &ContentManifest,
    workspace_root: &Path,
    override_base: Option<&Path>,
    usrdir_base: Option<&Path>,
    host: &mut Lv2Host,
) -> Result<ContentBaseSource, ContentRegisterError> {
    let (base, source) = if let Some(p) = override_base {
        // Relative override resolves against workspace_root for parity
        // with the manifest path.
        (
            resolve(workspace_root, &p.to_string_lossy()),
            ContentBaseSource::Override {
                env: manifest
                    .override_base_env
                    .clone()
                    .unwrap_or_else(|| "<no override_base_env declared>".to_string()),
            },
        )
    } else if let Some(usrdir) = usrdir_base.filter(|u| all_entries_resolve_under(u, manifest)) {
        (
            usrdir.to_path_buf(),
            ContentBaseSource::Usrdir {
                path: usrdir.to_path_buf(),
            },
        )
    } else {
        (
            resolve(workspace_root, &manifest.base),
            ContentBaseSource::Manifest,
        )
    };
    let override_env_for_err = match &source {
        ContentBaseSource::Override { env } => Some(env.clone()),
        ContentBaseSource::Manifest | ContentBaseSource::Usrdir { .. } => None,
    };
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
            // Surface both colliding host sources so the developer
            // sees the duplicate at a glance.
            let prior = manifest
                .files
                .iter()
                .find(|e| e.guest_path == entry.guest_path)
                .map(|e| resolve(&base, &e.host_path))
                .unwrap_or_else(|| host_path.clone());
            return Err(ContentRegisterError::DuplicateGuestPath {
                guest_path: entry.guest_path.clone(),
                first_host_path: prior,
                second_host_path: host_path,
            });
        }
    }
    Ok(source)
}

/// Look up the override base directory selected by a manifest's
/// `override_base_env`. Returns `Some(path)` when the env var is set
/// non-empty; `None` otherwise.
///
/// Takes a `getter` so tests can run without mutating process env.
pub fn override_base_from_env<F>(manifest: &ContentManifest, mut getter: F) -> Option<PathBuf>
where
    F: FnMut(&str) -> Option<String>,
{
    let env_name = manifest.override_base_env.as_deref()?;
    let value = getter(env_name)?;
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

#[cfg(test)]
#[path = "tests/content_tests.rs"]
mod tests;
