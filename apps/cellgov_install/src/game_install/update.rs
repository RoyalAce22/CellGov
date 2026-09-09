//! The update installer: one update PKG becomes one immutable store
//! entry at `titles/<ID>/updates/<version>/`, holding the `game/` tree
//! the update patches into `dev_hdd0/game/<ID>` at boot. The version
//! key is the version the PKG's PARAM.SFO names
//! ([`ParamSfo::named_version`]).
//!
//! Nothing here proves the installed EBOOT decrypts: an update may be
//! installed with no base present, and the klicensee that would
//! decrypt it comes from the base's RAP.
//!
//! [`ParamSfo::named_version`]: crate::param_sfo::ParamSfo::named_version

#![cfg_attr(
    not(feature = "decrypt"),
    allow(
        dead_code,
        unused_imports,
        reason = "the update installer is gated; the feature-on build lints these"
    )
)]

use std::path::{Path, PathBuf};

use crate::game_install::error::GameInstallError;
use crate::game_install::staging::{
    build_record, commit, dir_non_empty, emit_totals, io_err, parse_identity, prefixed_entry_path,
    prepare_staging, run_or_clean, stage_tree, validate_content_id, InstallOptions, StagedData,
    StagedFile,
};
use crate::keys::KeyVault;
use crate::pkg::{self, PkgEntryKind};
use crate::progress::Phase;
use crate::store::layout::{
    staging_sibling, Artifact, ArtifactKind, StoreLayout, TitleId, TitleTree, VersionKey,
};
use crate::store::lock::lock_artifact;
use crate::store::record::{ArtifactRecord, InstallRecord, TitleRecord};

/// PARAM.SFO categories an update PKG carries: `GD` patches a disc
/// title, `HG` patches an HDD title.
const UPDATE_CATEGORIES: [&str; 2] = ["GD", "HG"];

/// `distribution` tag every update record carries.
const UPDATE_DISTRIBUTION: &str = "update-pkg";

/// What a completed update install produced, for the CLI to report.
#[derive(Debug, Clone)]
pub struct UpdateInstallOutcome {
    /// PARAM.SFO `TITLE_ID` of the title this update patches.
    pub title_id: String,
    /// Full content id from the PKG header, or the title-id when the
    /// header carries none.
    pub content_id: String,
    /// The version key: the version the PKG's PARAM.SFO names, verbatim.
    pub version: String,
    /// The committed entry directory, holding the `game/` tree.
    pub update_dir: PathBuf,
    /// Whether the title has no base installed, so this update patches
    /// nothing.
    pub orphan: bool,
    /// Number of distinct files the committed tree holds.
    pub file_count: usize,
    /// The written install record.
    pub record_path: PathBuf,
    /// Whether `--force` replaced an already-installed version.
    pub replaced: bool,
}

/// Read an install record, distinguishing absence from a record that
/// is there and unreadable.
///
/// # Errors
///
/// [`GameInstallError::Io`] for a read failure other than absence, and
/// [`GameInstallError::RecordParse`] for a record this build refuses.
fn read_record(path: &Path) -> Result<Option<InstallRecord>, GameInstallError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_err("read", path)(e)),
    };
    InstallRecord::parse(&text)
        .map(Some)
        .map_err(|source| GameInstallError::RecordParse {
            path: path.to_path_buf(),
            source: Box::new(source),
        })
}

/// Hold the title entry the update is about to be written into: a base
/// record already there has to be this title's base.
///
/// Returns whether the update would be an orphan (no base installed).
///
/// # Errors
///
/// [`GameInstallError::BaseRecordMismatch`] when the base record names
/// another title or another kind of artifact.
fn check_base_entry(base_record_path: &Path, title_id: &str) -> Result<bool, GameInstallError> {
    let Some(base) = read_record(base_record_path)? else {
        return Ok(true);
    };
    let found = base.title.as_ref().map_or("", |t| t.title_id.as_str());
    if base.artifact.kind != ArtifactKind::TitleBase || found != title_id {
        return Err(GameInstallError::BaseRecordMismatch {
            path: base_record_path.to_path_buf(),
            kind: base.artifact.kind,
            found: found.to_string(),
            expected: title_id.to_string(),
        });
    }
    Ok(false)
}

/// Install one update PKG into `output_dir`'s versioned store, at
/// `titles/<TITLE_ID>/updates/<version>/`.
///
/// The entry is immutable: an already-installed version is refused by
/// name unless `opts.force`, which replaces it whole. No base is
/// required; an update installed without one is recorded as an orphan.
///
/// # Errors
///
/// [`GameInstallError::PreStore`] when the root still holds the
/// pre-store layout. Nothing is read or staged before this refusal.
///
/// [`GameInstallError::NotUpdatePackage`] for a category that is
/// neither `GD` nor `HG`, [`GameInstallError::MissingAppVersion`] when
/// nothing names the version directory,
/// [`GameInstallError::UpdateVersionInstalled`] for a duplicate,
/// [`GameInstallError::TargetExists`] for an unrecorded entry directory
/// that is not empty, [`GameInstallError::BaseRecordMismatch`] when the
/// base record under this title's entry is not this title's base, and
/// the shared container / staging / record failures.
///
/// [`GameInstallError::Locked`] when another process holds this version
/// of this title.
#[cfg(feature = "decrypt")]
pub fn install_update_pkg(
    pkg_bytes: &[u8],
    keys: &KeyVault,
    output_dir: &Path,
    opts: InstallOptions<'_>,
) -> Result<UpdateInstallOutcome, GameInstallError> {
    crate::store::pre_store::preflight(output_dir)?;
    let progress = opts.progress;
    progress.phase(Phase::Reading.code());
    let archive = pkg::extract(pkg_bytes, keys)?;

    let sfo_file = archive
        .files
        .iter()
        .find(|f| f.name == "PARAM.SFO")
        .ok_or(GameInstallError::NoParamSfo)?;
    let (title_id, category, title, version) = parse_identity(archive.file_data(sfo_file))?;
    if !UPDATE_CATEGORIES.contains(&category.as_str()) {
        return Err(GameInstallError::NotUpdatePackage { category });
    }
    // Same containment tie the base installer makes: the header's
    // content-id field embeds the 9-char title-id.
    if !archive.header.content_id.is_empty() && !archive.header.content_id.contains(&title_id) {
        return Err(GameInstallError::TitleIdMismatch {
            header: archive.header.content_id.clone(),
            sfo: title_id,
        });
    }
    let content_id = if archive.header.content_id.is_empty() {
        title_id.clone()
    } else {
        archive.header.content_id.clone()
    };
    validate_content_id(&title_id)?;
    validate_content_id(&content_id)?;

    if version.is_empty() {
        return Err(GameInstallError::MissingAppVersion);
    }
    let key = TitleId::new(&title_id)?;
    let artifact = Artifact::TitleUpdate {
        title_id: key.clone(),
        version: VersionKey::new(&version)?,
    };
    let layout = StoreLayout::new(output_dir);
    let entry_dir = layout.entry_dir(&artifact);
    let store_path = layout.store_path_of(&entry_dir)?;
    let record_path = layout.record_path(&artifact);
    let staging_root = staging_sibling(&entry_dir)?;
    // The install holds this claim to the end, as on the base path. The
    // claim covers this version alone. Another version of the same title
    // stages and commits under its own key.
    let _lock = lock_artifact(&layout, &artifact)?;

    let orphan = check_base_entry(
        &layout.record_path(&Artifact::TitleBase { title_id: key }),
        &title_id,
    )?;

    // Records are the store's index, so an installed version is one
    // with a record; a tree with no record is residue the target gate
    // refuses on its own terms.
    let existing = read_record(&record_path)?;
    if let Some(existing) = &existing {
        if !opts.force {
            return Err(GameInstallError::UpdateVersionInstalled {
                version,
                existing_source: existing.source.sha256,
            });
        }
    } else if dir_non_empty(&entry_dir)? && !opts.force {
        return Err(GameInstallError::TargetExists { path: entry_dir });
    }

    // The entry directory holds the tree under `game/`, so the staged
    // paths carry that prefix and the record's `[files]` keys stay
    // relative to the entry the record names.
    let tree_dir = TitleTree::Game.dir_name();
    let staged: Vec<StagedFile> = archive
        .files
        .iter()
        .map(|f| {
            Ok(StagedFile {
                path: prefixed_entry_path(tree_dir, &f.name)?,
                is_dir: f.kind == PkgEntryKind::Directory,
                data: StagedData::Bytes(archive.file_data(f)),
            })
        })
        .collect::<Result<_, GameInstallError>>()?;
    emit_totals(progress, &staged);

    prepare_staging(&staging_root, progress)?;
    let file_digests = run_or_clean(&staging_root, || {
        progress.phase(Phase::Staging.code());
        stage_tree(&staged, &staging_root, progress)
    })?;

    progress.phase(Phase::Hashing.code());
    let record = build_record(
        "pkg",
        pkg_bytes,
        ArtifactRecord {
            kind: artifact.kind(),
            version: version.clone(),
            store_path,
        },
        file_digests,
        TitleRecord {
            title_id: title_id.clone(),
            content_id: content_id.clone(),
            category,
            title,
            distribution: UPDATE_DISTRIBUTION.to_string(),
        },
        None,
    );
    // The staging root is itself the entry directory, as on the disc
    // path; there is no RAP.
    let record_path = commit(
        &staging_root,
        &staging_root,
        &entry_dir,
        None,
        &record_path,
        &record,
        progress,
    )?;

    progress.finished();
    Ok(UpdateInstallOutcome {
        title_id,
        content_id,
        version,
        update_dir: entry_dir,
        orphan,
        file_count: record.files.len(),
        record_path,
        replaced: existing.is_some(),
    })
}

#[cfg(test)]
#[path = "tests/update_tests.rs"]
mod tests;
