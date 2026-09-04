//! Why an uninstall was refused.

use std::path::{Path, PathBuf};

use crate::manifest::Sha256 as HexSha256;
use crate::store::layout::ArtifactKind;
use crate::store::verify::VerifyReadError;

/// Why an uninstall failed.
#[derive(Debug, thiserror::Error)]
pub enum GameUninstallError {
    /// The pre-store check refused the root.
    #[error("{0}")]
    PreStore(#[from] crate::store::pre_store::PreStoreError),
    /// The recorded tree names no tombstone sibling to rename onto.
    #[error("{0}")]
    HiddenSibling(#[from] crate::store::layout::HiddenSiblingError),
    /// The title has no base install record. An update installs with no
    /// base present, so this does not say the title holds no entry.
    #[error("no base install record for title {title_id:?}")]
    NoRecord {
        /// The requested title-id.
        title_id: String,
    },
    /// No record exists for the update version the caller named.
    #[error("title {title_id:?} has no installed update {version:?}; installed: {installed}")]
    NoUpdateRecord {
        /// The requested title-id.
        title_id: String,
        /// The requested update version.
        version: String,
        /// The versions that are installed, comma-separated, or
        /// `<none>`.
        installed: String,
    },
    /// The bare title-id names the base alone, and removing it would
    /// leave its updates patching nothing.
    #[error(
        "title {title_id:?} has {} installed update(s) ({updates}); removing the base alone \
         leaves them patching nothing. Pass --all to remove the title, or --updates to remove \
         the updates and keep the base",
        count
    )]
    UpdatesInstalled {
        /// The requested title-id.
        title_id: String,
        /// How many updates are installed.
        count: usize,
        /// The installed update versions, comma-separated.
        updates: String,
    },
    /// Reading the install record failed (for a reason other than absence).
    #[error("read install record {}: {source}", path.display())]
    RecordRead {
        /// The record path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// Enumerating a title's records failed.
    #[error("reading the install records under {}: {source}", dir.display())]
    RecordsReadDir {
        /// The directory that could not be enumerated.
        dir: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// Loading the install record failed: bad TOML, or a schema this
    /// build does not read.
    #[error("parse install record: {0}")]
    RecordParse(#[from] crate::store::record::InstallRecordParseError),
    /// The operator-supplied title-id is not usable as a single path
    /// component under the store roots, so it names no record.
    #[error("unsafe title-id {title_id:?}")]
    UnsafeTitleId {
        /// The offending title-id.
        title_id: String,
        /// Which rule it broke.
        #[source]
        source: crate::store::layout::StoreKeyError,
    },
    /// The version the caller named is not usable as a store directory
    /// name, so it names no entry.
    #[error("unsafe update version {version:?}")]
    UnsafeVersion {
        /// The offending version.
        version: String,
        /// Which rule it broke.
        #[source]
        source: crate::store::layout::StoreKeyError,
    },
    /// The record filed under this title describes some other kind of
    /// store entry, so nothing here names a title tree to remove.
    #[error(
        "install record for {title_id:?} describes a {} entry, not a {}",
        found.as_str(),
        expected.as_str()
    )]
    RecordKindMismatch {
        /// The requested title-id.
        title_id: String,
        /// The kind the entry's own record must declare.
        expected: ArtifactKind,
        /// The kind the record declared.
        found: ArtifactKind,
    },
    /// The record's `store_path` names a tree that is not this title's.
    #[error(
        "install record for {title_id:?} names the tree {store_path:?}, which is not this title's"
    )]
    RecordTreeForeign {
        /// The requested title-id.
        title_id: String,
        /// The `store_path` the record declared.
        store_path: String,
    },
    /// A recorded file could be neither hashed nor shown absent, so the
    /// gate cannot say whether the tree matches.
    #[error("{0}")]
    VerifyRead(#[from] VerifyReadError),
    /// A file the record lists is absent from the live tree; pass
    /// `force` to uninstall anyway.
    #[error("recorded file missing from the installed tree: {}", path.display())]
    RecordedFileMissing {
        /// The absent path.
        path: PathBuf,
    },
    /// A live file's hash diverged from the record (the tree was
    /// modified since install); pass `force` to uninstall anyway.
    #[error("tree modified since install: {} (recorded {}, found {})", path.display(), expected.to_hex(), found.to_hex())]
    TreeModified {
        /// The diverging path.
        path: PathBuf,
        /// Hash the record holds.
        expected: HexSha256,
        /// Hash found on disk.
        found: HexSha256,
    },
    /// A filesystem operation failed.
    #[error("{op} {}: {source}", path.display())]
    Io {
        /// What was being attempted.
        op: &'static str,
        /// The path involved.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
}

pub(super) fn uio_err<'a>(
    op: &'static str,
    path: &'a Path,
) -> impl Fn(std::io::Error) -> GameUninstallError + 'a {
    move |source| GameUninstallError::Io {
        op,
        path: path.to_path_buf(),
        source,
    }
}

/// A version list as a refusal renders it.
pub(super) fn render_versions(versions: &[String]) -> String {
    if versions.is_empty() {
        "<none>".to_string()
    } else {
        versions.join(", ")
    }
}
