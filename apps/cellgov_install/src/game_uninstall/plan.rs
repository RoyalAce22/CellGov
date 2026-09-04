//! What a scope removes, resolved from the records before the teardown
//! touches anything.
//!
//! [`super::uninstall`] executes the plan [`plan`] returns, so a
//! `--dry-run` listing and the removal describe the same set.

use std::path::{Path, PathBuf};

use crate::store::layout::{Artifact, StoreLayout, TitleId, VersionKey};
use crate::store::record::InstallRecord;

use super::error::{render_versions, GameUninstallError};

/// Which of a title's entries an uninstall removes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UninstallScope {
    /// The base tree alone. [`plan`] refuses it while updates are
    /// installed, since removing the base leaves them patching nothing.
    Base,
    /// One installed update version.
    Update(String),
    /// Every installed update, keeping the base.
    Updates,
    /// Every update and the base.
    All,
}

/// Which entry of a title one planned removal names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryVersion {
    /// The title's base install.
    Base,
    /// One update version.
    Update(String),
}

impl std::fmt::Display for EntryVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Base => f.write_str("base"),
            Self::Update(v) => write!(f, "update {v}"),
        }
    }
}

/// One store entry a planned uninstall removes.
#[derive(Debug, Clone)]
pub struct PlannedEntry {
    /// Which entry of the title this is.
    pub version: EntryVersion,
    /// The directory removed whole.
    pub tree_dir: PathBuf,
    /// The record removed with it.
    pub record_path: PathBuf,
    /// Files the record lists for this entry.
    pub recorded_files: usize,
    /// The record itself, so the removal need not read it twice.
    pub(super) record: InstallRecord,
    /// The identity the removal claims, from the gate that resolved
    /// `tree_dir`.
    pub(super) artifact: Artifact,
}

/// Every entry a scope removes, updates first and the base last.
#[derive(Debug, Clone)]
pub struct UninstallPlan {
    /// The title the scope named.
    pub title_id: String,
    /// The entries to remove, in removal order.
    pub entries: Vec<PlannedEntry>,
    /// The RAP the base record names, when the plan removes the base.
    pub rap: Option<PathBuf>,
    /// Installed update versions this scope leaves in place.
    pub kept_updates: Vec<String>,
    /// The root the plan resolved against, so the removal claims its
    /// locks under the same store.
    pub(super) layout: StoreLayout,
}

impl UninstallPlan {
    /// Files every entry in the plan records.
    #[must_use]
    pub fn recorded_files(&self) -> usize {
        self.entries.iter().map(|e| e.recorded_files).sum()
    }
}

/// Resolve what `scope` removes from `title_id` under `output_dir`.
///
/// The plan reads records only and changes nothing on disk.
///
/// # Errors
///
/// - [`GameUninstallError::PreStore`] when the root still holds the
///   pre-store layout.
/// - [`GameUninstallError::NoRecord`] when the title has no base and
///   the scope needs one. [`UninstallScope::Updates`] over an id that
///   names no entry at all refuses here rather than reading as a
///   removal.
/// - [`GameUninstallError::NoUpdateRecord`] for an update version that
///   is not installed.
/// - [`GameUninstallError::UpdatesInstalled`] when
///   [`UninstallScope::Base`] would orphan an update.
///
/// The record read, parse, and identity refusals also apply.
pub fn plan(
    title_id: &str,
    output_dir: &Path,
    scope: &UninstallScope,
) -> Result<UninstallPlan, GameUninstallError> {
    crate::store::pre_store::preflight(output_dir)?;
    let key = TitleId::new(title_id).map_err(|source| GameUninstallError::UnsafeTitleId {
        title_id: title_id.to_string(),
        source,
    })?;
    let layout = StoreLayout::new(output_dir);
    let installed = installed_updates(&layout, &key)?;

    let (wanted_updates, wants_base) = match scope {
        UninstallScope::Base => {
            if !installed.is_empty() {
                return Err(GameUninstallError::UpdatesInstalled {
                    title_id: title_id.to_string(),
                    count: installed.len(),
                    updates: render_versions(&installed),
                });
            }
            (Vec::new(), true)
        }
        UninstallScope::Update(version) => {
            if !installed.iter().any(|v| v == version) {
                return Err(GameUninstallError::NoUpdateRecord {
                    title_id: title_id.to_string(),
                    version: version.clone(),
                    installed: render_versions(&installed),
                });
            }
            (vec![version.clone()], false)
        }
        UninstallScope::Updates => {
            if installed.is_empty() {
                require_base_record(&layout, &key, title_id)?;
            }
            (installed.clone(), false)
        }
        UninstallScope::All => (installed.clone(), true),
    };

    let mut entries = Vec::with_capacity(wanted_updates.len() + usize::from(wants_base));
    for version in &wanted_updates {
        let version_key =
            VersionKey::new(version).map_err(|source| GameUninstallError::UnsafeVersion {
                version: version.clone(),
                source,
            })?;
        let artifact = Artifact::TitleUpdate {
            title_id: key.clone(),
            version: version_key,
        };
        entries.push(planned_entry(
            title_id,
            &layout,
            &artifact,
            EntryVersion::Update(version.clone()),
        )?);
    }

    let mut rap = None;
    if wants_base {
        let artifact = Artifact::TitleBase {
            title_id: key.clone(),
        };
        let entry = planned_entry(title_id, &layout, &artifact, EntryVersion::Base)?;
        // The RAP is keyed by content id in the live exdata directory,
        // which no store entry owns.
        rap = entry
            .record
            .rap
            .as_ref()
            .map(|r| layout.live_exdata_dir().join(&r.filename));
        entries.push(entry);
    }

    let kept_updates = installed
        .into_iter()
        .filter(|v| !wanted_updates.contains(v))
        .collect();
    Ok(UninstallPlan {
        title_id: title_id.to_string(),
        entries,
        rap,
        kept_updates,
        layout,
    })
}

/// Refuse an id that names no store entry at all.
///
/// A title with a base and no update has nothing for
/// [`UninstallScope::Updates`] to take. An id that names no record is a
/// miss, which every other scope also refuses by name.
fn require_base_record(
    layout: &StoreLayout,
    key: &TitleId,
    title_id: &str,
) -> Result<(), GameUninstallError> {
    let path = layout.record_path(&Artifact::TitleBase {
        title_id: key.clone(),
    });
    let found = std::fs::metadata(&path);
    match found {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(GameUninstallError::NoRecord {
            title_id: title_id.to_string(),
        }),
        Err(source) => Err(GameUninstallError::RecordRead { path, source }),
    }
}

/// Load and gate one entry's record.
fn planned_entry(
    title_id: &str,
    layout: &StoreLayout,
    artifact: &Artifact,
    version: EntryVersion,
) -> Result<PlannedEntry, GameUninstallError> {
    let record_path = layout.record_path(artifact);
    let record = read_record(&record_path, title_id, &version)?;
    check_record_describes(title_id, layout, artifact, &record)?;
    Ok(PlannedEntry {
        version,
        tree_dir: layout.resolve_store_path(&record.artifact.store_path),
        record_path,
        recorded_files: record.files.len(),
        record,
        artifact: artifact.clone(),
    })
}

/// Re-read `entry`'s record under the claim and hold it against what
/// [`plan`] resolved.
///
/// [`plan`] reads every record before any claim exists. The CLI prints
/// the plan and waits for an operator answer, so the gap runs as long as
/// the operator takes. A base record names a live mount directory, and
/// [`check_record_describes`] asks only that its last component be the
/// title-id. An install that lands the same title on another mount in
/// that gap moves the tree but leaves the record path alone. The removal
/// would then take the tree the plan named and delete the record the new
/// install wrote. Nothing would name the tree that stays installed.
pub(super) fn recheck_under_claim(
    layout: &StoreLayout,
    title_id: &str,
    entry: &PlannedEntry,
) -> Result<(), GameUninstallError> {
    let record = read_record(&entry.record_path, title_id, &entry.version)?;
    check_record_describes(title_id, layout, &entry.artifact, &record)?;
    let named = layout.resolve_store_path(&record.artifact.store_path);
    if named != entry.tree_dir {
        return Err(GameUninstallError::RecordMovedSincePlan {
            path: entry.record_path.clone(),
            planned: entry.tree_dir.clone(),
            found: named,
        });
    }
    Ok(())
}

fn read_record(
    path: &Path,
    title_id: &str,
    version: &EntryVersion,
) -> Result<InstallRecord, GameUninstallError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(match version {
                EntryVersion::Base => GameUninstallError::NoRecord {
                    title_id: title_id.to_string(),
                },
                // The enumeration named this version from its own
                // record file, so a miss here means it went away between
                // the walk and the read.
                EntryVersion::Update(v) => GameUninstallError::NoUpdateRecord {
                    title_id: title_id.to_string(),
                    version: v.clone(),
                    installed: render_versions(&[]),
                },
            });
        }
        Err(source) => {
            return Err(GameUninstallError::RecordRead {
                path: path.to_path_buf(),
                source,
            })
        }
    };
    Ok(InstallRecord::parse(&text)?)
}

/// Refuse a record that does not describe the entry the caller named.
///
/// The parse gate proves `store_path` stays under the VFS root, but not
/// which tree it names. The recorded path steers the tombstone rename
/// and the `remove_dir_all` the removal runs, and under that root it may
/// name:
///
/// - another title's tree,
/// - another version's tree,
/// - a whole mount.
fn check_record_describes(
    title_id: &str,
    layout: &StoreLayout,
    artifact: &Artifact,
    record: &InstallRecord,
) -> Result<(), GameUninstallError> {
    let expected = artifact.kind();
    if record.artifact.kind != expected {
        return Err(GameUninstallError::RecordKindMismatch {
            title_id: title_id.to_string(),
            expected,
            found: record.artifact.kind,
        });
    }
    let named = layout.resolve_store_path(&record.artifact.store_path);
    let describes_this_entry = match artifact {
        // The store keys an update entry on (id, version), so its record
        // must name the directory that key resolves to.
        // `firmware_uninstall::check_record_describes` makes the same
        // tie for a firmware entry.
        Artifact::Firmware { .. } | Artifact::TitleUpdate { .. } => {
            named == layout.entry_dir(artifact)
        }
        // A base tree is a live mount directory the installers name
        // after the title (`dev_hdd0/game/<id>` from a PKG,
        // `dev_bdvd/<id>` from a disc image), so the id is its final
        // component. A parent of that directory is not this record's to
        // remove.
        Artifact::TitleBase { .. } => named.file_name().and_then(|n| n.to_str()) == Some(title_id),
    };
    if !describes_this_entry {
        return Err(GameUninstallError::RecordTreeForeign {
            title_id: title_id.to_string(),
            store_path: record.artifact.store_path.clone(),
        });
    }
    Ok(())
}

/// Prefix an update record's filename carries before its version key.
const UPDATE_RECORD_PREFIX: &str = "update-";

/// Suffix every install-record filename carries.
const INSTALL_RECORD_SUFFIX: &str = ".install.toml";

/// The update versions installed for `title_id`.
///
/// A version key is never normalized, so the keys sort in byte order:
/// `1.10` comes before `1.9`.
fn installed_updates(
    layout: &StoreLayout,
    title_id: &TitleId,
) -> Result<Vec<String>, GameUninstallError> {
    let dir = layout.installs_dir().join("titles").join(title_id.as_str());
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(GameUninstallError::RecordsReadDir { dir, source }),
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| GameUninstallError::RecordsReadDir {
            dir: dir.clone(),
            source,
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(rest) = name.strip_prefix(UPDATE_RECORD_PREFIX) {
            if let Some(version) = rest.strip_suffix(INSTALL_RECORD_SUFFIX) {
                out.push(version.to_string());
            }
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
#[path = "tests/plan_tests.rs"]
mod tests;
