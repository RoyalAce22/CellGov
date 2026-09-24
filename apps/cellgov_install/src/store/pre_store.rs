//! Detection of the layout CellGov used before the versioned store.
//!
//! The store resolver reads one layout. This module refuses a root that
//! still holds the older one. The refusal names the residue and the
//! command that rebuilds it. There is no migration path.
//!
//! Only what the store no longer writes counts as residue. A title's
//! base tree stays guest-visible at `dev_hdd0/game` and `dev_bdvd`, and
//! its record names it through `store_path`, so the record's location
//! is the marker.

use std::path::{Path, PathBuf};

use cellgov_ps3_abi::format::dev_flash::FLASH_MOUNTS;

use crate::store::inventory::{record_files, RecordDirError};
use crate::store::layout::StoreLayout;

/// What a piece of pre-store residue was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreStoreArtifact {
    /// One of the flash mounts every firmware install shared, before
    /// the store keyed firmware entries on version.
    FirmwareMount,
    /// A `<title-id>.install.toml` directly under the records
    /// directory, which the store files under `titles/<id>/` instead.
    FlatInstallRecord,
}

impl PreStoreArtifact {
    fn describe(self) -> &'static str {
        match self {
            Self::FirmwareMount => "a single-version firmware mount",
            Self::FlatInstallRecord => "an unversioned install record",
        }
    }

    /// The command that rebuilds it under the store layout.
    fn rebuild_with(self) -> &'static str {
        match self {
            Self::FirmwareMount => "`cellgov firmware install <PS3UPDAT.PUP>`",
            Self::FlatInstallRecord => "`cellgov title install <PKG|ISO>`",
        }
    }
}

/// One piece of pre-store residue under a store root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreStoreResidue {
    /// Where it is.
    pub path: PathBuf,
    /// What it was.
    pub artifact: PreStoreArtifact,
}

/// Why a store root could not be used.
#[derive(Debug, thiserror::Error)]
pub enum PreStoreError {
    /// The root still holds the layout that came before the store.
    #[error(
        "{} holds the layout CellGov used before the versioned store -- {}; there is no \
         migration path, so remove what is named and rebuild the store with {}",
        .root.display(),
        render_residue(.root, .residue),
        render_rebuild(.residue)
    )]
    Residue {
        /// The store root that was probed.
        root: PathBuf,
        /// Every piece of residue found, mounts first, then records by
        /// name.
        residue: Vec<PreStoreResidue>,
    },
    /// A probe could not answer, so nothing downstream can say which
    /// layout the root holds.
    #[error("cannot tell which layout {} holds: probing {}: {source}", .root.display(), .probed.display())]
    Probe {
        /// The store root that was probed.
        root: PathBuf,
        /// The path whose probe failed.
        probed: PathBuf,
        /// Why it failed.
        #[source]
        source: std::io::Error,
    },
}

/// `<path under the root> (<what it was>)`, comma-joined.
fn render_residue(root: &Path, residue: &[PreStoreResidue]) -> String {
    residue
        .iter()
        .map(|r| {
            let shown = r.path.strip_prefix(root).unwrap_or(&r.path);
            format!("{} ({})", shown.display(), r.artifact.describe())
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The distinct rebuild commands the residue calls for, in first-seen
/// order.
fn render_rebuild(residue: &[PreStoreResidue]) -> String {
    let mut commands: Vec<&str> = Vec::new();
    for r in residue {
        let command = r.artifact.rebuild_with();
        if !commands.contains(&command) {
            commands.push(command);
        }
    }
    commands.join(" and ")
}

/// Refuse `root` when it still holds the pre-store layout.
///
/// # Errors
///
/// - [`PreStoreError::Residue`] when the root holds residue. It names
///   every piece found.
/// - [`PreStoreError::Probe`] when a probe can neither find residue nor
///   show its absence.
pub fn preflight(root: &Path) -> Result<(), PreStoreError> {
    let residue = detect(root)?;
    if residue.is_empty() {
        Ok(())
    } else {
        Err(PreStoreError::Residue {
            root: root.to_path_buf(),
            residue,
        })
    }
}

/// Every piece of pre-store residue under `root`: the flash mounts in
/// [`FLASH_MOUNTS`] order, then the flat records sorted by name.
fn detect(root: &Path) -> Result<Vec<PreStoreResidue>, PreStoreError> {
    let mut residue = Vec::new();
    for mount in FLASH_MOUNTS {
        let dir = root.join(mount);
        if probe(root, &dir)?.is_some_and(|md| md.is_dir()) {
            residue.push(PreStoreResidue {
                path: dir,
                artifact: PreStoreArtifact::FirmwareMount,
            });
        }
    }
    let mut records = flat_records(root, &StoreLayout::new(root).installs_dir())?;
    records.sort();
    residue.extend(records.into_iter().map(|path| PreStoreResidue {
        path,
        artifact: PreStoreArtifact::FlatInstallRecord,
    }));
    Ok(residue)
}

/// Record files directly under `installs`, where the store writes none:
/// a firmware record sits under `firmware/`, a title record under
/// `titles/<id>/`.
fn flat_records(root: &Path, installs: &Path) -> Result<Vec<PathBuf>, PreStoreError> {
    let named_like_a_record =
        record_files(installs).map_err(|RecordDirError { dir, source }| PreStoreError::Probe {
            root: root.to_path_buf(),
            probed: dir,
            source,
        })?;
    let mut out = Vec::new();
    for path in named_like_a_record {
        if probe(root, &path)?.is_some_and(|md| md.is_file()) {
            out.push(path);
        }
    }
    Ok(out)
}

/// `path`'s metadata, or `None` when it does not exist.
///
/// # Errors
///
/// [`PreStoreError::Probe`] for every failure except absence.
fn probe(root: &Path, path: &Path) -> Result<Option<std::fs::Metadata>, PreStoreError> {
    match std::fs::metadata(path) {
        Ok(md) => Ok(Some(md)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(PreStoreError::Probe {
            root: root.to_path_buf(),
            probed: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
#[path = "tests/pre_store_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/preflight_wiring_tests.rs"]
mod preflight_wiring_tests;
