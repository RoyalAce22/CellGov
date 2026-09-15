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

/// Why `register_content_blobs` could not register a manifest's
/// content.
#[derive(Debug, thiserror::Error)]
pub enum ContentRegisterError {
    /// Reading the host file failed (NotFound, permission-denied, IO).
    #[error(
        "content: failed to read host file {} for guest path {:?}: {source}{}{}",
        host_path.display(),
        guest_path,
        render_also_probed(also_probed),
        render_override_hint(override_env)
    )]
    HostFileRead {
        /// The guest path the entry declares.
        guest_path: String,
        /// The path the error names:
        ///
        /// - the candidate under the first base, when every base lacks
        ///   the file
        /// - the candidate whose read failed, otherwise
        host_path: PathBuf,
        /// The candidates the probe found absent before it reached
        /// `host_path`.
        also_probed: Vec<PathBuf>,
        /// What the read refused with.
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
        /// The guest path both entries declare.
        guest_path: String,
        /// The host file the first entry read from.
        first_host_path: PathBuf,
        /// The host file the second entry read from.
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
        /// The guest path the entry declares.
        guest_path: String,
        /// The host file the entry names.
        host_path: PathBuf,
    },
    /// No base directory: the override env var is unset or empty, and
    /// the caller gave no EBOOT directory.
    #[error(
        "content: no base directory for {n} manifest entr{}: {} and the composition names \
         no EBOOT directory",
        if *n == 1 { "y" } else { "ies" },
        render_no_override(override_env)
    )]
    NoBase {
        /// How many manifest entries have no base to read from.
        n: usize,
        /// The override env var the manifest declares, if any.
        override_env: Option<String>,
    },
}

fn render_also_probed(also_probed: &[PathBuf]) -> String {
    if also_probed.is_empty() {
        return String::new();
    }
    let list = also_probed
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!(" (also absent under: {list})")
}

fn render_override_hint(override_env: &Option<String>) -> String {
    match override_env {
        Some(env) => format!(
            " (override env var {env} is set; either drop the \
             file into that directory or unset {env} to read \
             from the composition's EBOOT directories)"
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

/// Source of the resolved content base directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentBaseSource {
    /// The EBOOT directories in probe order, taken as given; a selected
    /// update's directory comes first.
    Usrdir {
        /// The EBOOT directories, in probe order.
        paths: Vec<PathBuf>,
    },
    /// Override env var named by `[content] override_base_env`.
    Override {
        /// The env var that names the base.
        env: String,
    },
}

/// Read each manifest entry off disk and register the bytes in
/// `host.fs_store_mut`.
///
/// The base directories come from the first of these that is present:
///
/// 1. `override_base`, joined onto `workspace_root` when relative
/// 2. `usrdir_bases`, taken as given, in order
///
/// A relative `host_path` resolves against each base in turn, and the
/// first base that holds the file supplies it.
///
/// # Errors
///
/// - [`ContentRegisterError::NoBase`]: no override and no EBOOT
///   directory.
/// - [`ContentRegisterError::HostFileRead`]: every base lacks a file,
///   or a read under one base failed. The error names that path and
///   the other candidates it probed.
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
    usrdir_bases: &[PathBuf],
    host: &mut Lv2Host,
) -> Result<ContentBaseSource, ContentRegisterError> {
    let (bases, source) = match override_base {
        // `Path::join` keeps a rooted `p` as is, so a relative override
        // resolves against `workspace_root` and an absolute one passes
        // through.
        Some(p) => (
            vec![workspace_root.join(p)],
            ContentBaseSource::Override {
                env: manifest
                    .override_base_env
                    .clone()
                    .unwrap_or_else(|| "<no override_base_env declared>".to_string()),
            },
        ),
        None if !usrdir_bases.is_empty() => (
            usrdir_bases.to_vec(),
            ContentBaseSource::Usrdir {
                paths: usrdir_bases.to_vec(),
            },
        ),
        None => {
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
        let (host_path, bytes) = read_under_first(&bases, &entry.host_path).map_err(|refusal| {
            ContentRegisterError::HostFileRead {
                guest_path: entry.guest_path.clone(),
                host_path: refusal.host_path,
                also_probed: refusal.also_probed,
                source: refusal.source,
                override_env: override_env_for_err.clone(),
            }
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

/// Why no base supplied a content file: the fields
/// [`ContentRegisterError::HostFileRead`] carries for it.
struct ProbeRefusal {
    host_path: PathBuf,
    also_probed: Vec<PathBuf>,
    source: std::io::Error,
}

/// Read `host_path` under the first of `bases` that holds it.
///
/// When a base lacks the file ([`is_absent_under_base`]), the probe
/// continues to the next base. Any other read failure stops the probe
/// at that base. When every base lacks the file, the error names the
/// first base's candidate and lists the rest after it.
///
/// # Errors
///
/// [`ProbeRefusal`], when no base supplies the file.
fn read_under_first(
    bases: &[PathBuf],
    host_path: &str,
) -> Result<(PathBuf, Vec<u8>), ProbeRefusal> {
    // An absolute `host_path` resolves the same under every base, so
    // the probe reads it once.
    let mut candidates: Vec<PathBuf> = Vec::with_capacity(bases.len());
    for base in bases {
        let candidate = resolve(base, host_path);
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    let mut absent: Vec<(PathBuf, std::io::Error)> = Vec::new();
    for candidate in candidates {
        match std::fs::read(&candidate) {
            Ok(bytes) => return Ok((candidate, bytes)),
            Err(e) if is_absent_under_base(&e) => absent.push((candidate, e)),
            Err(e) => {
                return Err(ProbeRefusal {
                    host_path: candidate,
                    also_probed: absent.into_iter().map(|(p, _)| p).collect(),
                    source: e,
                });
            }
        }
    }
    let mut absent = absent.into_iter();
    let (host_path, source) = absent
        .next()
        .expect("invariant: the caller passes at least one base");
    Err(ProbeRefusal {
        host_path,
        also_probed: absent.map(|(p, _)| p).collect(),
        source,
    })
}

/// The read error kinds that mean the base holds nothing at that name.
///
/// The mount layer's per-root probe (`cellgov_lv2::host::fs::mount`,
/// `probe`) reads the same set as a miss. A content entry and a
/// hostless mount therefore fall through the same bases:
///
/// - `NotFound`.
/// - `NotADirectory`, from a host family where a path component is a
///   regular file; another family reports that case as `NotFound`.
/// - `InvalidFilename`, from a host that cannot express the name.
///
/// Every other kind leaves the base's contents unknown.
fn is_absent_under_base(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::NotFound
            | std::io::ErrorKind::NotADirectory
            | std::io::ErrorKind::InvalidFilename
    )
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

#[cfg(test)]
#[path = "tests/content_bases_tests.rs"]
mod bases_tests;
