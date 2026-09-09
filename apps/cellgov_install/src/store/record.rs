//! The install-record schema: one TOML file per store entry, and the
//! store's only index.
//!
//! # Invariants
//!
//! - A record's blocks match its `[artifact] kind`: a firmware record
//!   has no `[title]` and no `[rap]`, a title record has a `[title]`.
//!   [`InstallRecord`] has no `Deserialize`; [`InstallRecord::parse`] is
//!   the only route from text and refuses every other combination.
//! - Every string a consumer joins onto a path -- `store_path`, the
//!   `[files]` keys, the `[rap]` filename, the `[title]` ids -- is gated
//!   here, so no record read off disk can name a file outside the tree
//!   it describes.
//! - The version of a kind whose store path encodes one (firmware,
//!   update) is a usable directory name. A base's version is the one
//!   its PARAM.SFO names, which the path does not encode and a container
//!   may leave empty.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::manifest::Sha256 as HexSha256;
use crate::store::layout::{is_safe_component, store_path_is_safe, ArtifactKind};

/// Install-record schema version; a record declaring any other is refused.
pub const INSTALL_RECORD_FORMAT_VERSION: u32 = 3;

/// Why an install record could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum InstallRecordParseError {
    /// The file is not valid TOML, or does not match the record shape.
    #[error("install record is not valid TOML: {0}")]
    Toml(#[from] toml::de::Error),
    /// The record declares a schema this build does not read.
    #[error(
        "install record declares format_version {found}, this build reads {supported}; \
         there is no migration path, so remove it and reinstall with \
         `cellgov firmware install <PS3UPDAT.PUP>` or `cellgov title install <PKG|ISO>`"
    )]
    UnsupportedFormatVersion {
        /// Version the record declared.
        found: u32,
        /// The only version this build accepts.
        supported: u32,
    },
    /// A title record carries no `[title]` block, so nothing names the
    /// title it belongs to.
    #[error("{} record has no [title] block", kind.as_str())]
    MissingTitleBlock {
        /// The kind that requires one.
        kind: ArtifactKind,
    },
    /// A firmware record carries a `[title]` block.
    #[error("firmware record carries a [title] block")]
    UnexpectedTitleBlock,
    /// A firmware record carries a `[rap]` block.
    #[error("firmware record carries a [rap] block")]
    UnexpectedRapBlock,
    /// A firmware record carries a `[files]` table; the per-file
    /// manifest for a firmware tree is the `firmware.toml` inside it.
    #[error("firmware record carries a [files] table")]
    UnexpectedFilesBlock,
    /// A version the store path encodes is not a usable directory name.
    #[error(
        "{} record declares version {version:?}, which is not a store directory name",
        kind.as_str()
    )]
    UnsafeArtifactVersion {
        /// The kind whose store path encodes the version.
        kind: ArtifactKind,
        /// The offending version.
        version: String,
    },
    /// A `store_path` that is absolute, or whose components could leave
    /// the VFS root the record lives in.
    #[error("store_path {path:?} is not a relative path under the VFS root")]
    UnsafeStorePath {
        /// The offending path.
        path: String,
    },
    /// A `[title]` key that a consumer joins onto a path -- the store
    /// directory name, or the registry filename a manifest stub is
    /// written to -- and that is not a usable component.
    #[error("[title] {field} {value:?} is not usable as a path component")]
    UnsafeTitleKey {
        /// Which of `title_id` / `content_id` is at fault.
        field: &'static str,
        /// The offending value.
        value: String,
    },
    /// A `[rap] filename` that is not a usable component. Uninstall
    /// joins it onto the live exdata directory and removes the result.
    #[error("[rap] filename {filename:?} is not usable as a path component")]
    UnsafeRapFilename {
        /// The offending filename.
        filename: String,
    },
    /// A `[files]` key that could name a file outside the tree it is
    /// joined onto, or that aliases another key onto one file.
    #[error("[files] key {path:?} is not a path inside the recorded tree")]
    UnsafeFilePath {
        /// The offending key.
        path: String,
    },
}

/// The store entry a record describes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRecord {
    /// What the entry holds.
    pub kind: ArtifactKind,
    /// Sony's version string verbatim: for a title entry, the version
    /// its PARAM.SFO names ([`ParamSfo::named_version`]). The record
    /// does not say which key named it; read the tree's own PARAM.SFO
    /// for the key. A base's store path does not encode it.
    ///
    /// [`ParamSfo::named_version`]: crate::param_sfo::ParamSfo::named_version
    pub version: String,
    /// The entry's directory, `/`-separated and relative to the VFS
    /// root -- where the tree actually is, which a move rewrites.
    pub store_path: String,
}

/// Source-container provenance for an install record.
///
/// The acquisition fields are present only for a container fetched off
/// the wire, so their presence is what marks one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRecord {
    /// Container kind: `pkg`, `iso`, or `pup`. No consumer joins it
    /// onto a path, so the parse gate leaves it ungated.
    pub kind: String,
    /// SHA-256 over the source container bytes.
    pub sha256: HexSha256,
    /// URL the container was fetched from.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub url: Option<String>,
    /// When it was fetched, RFC 3339 wall time; records are operator
    /// artifacts outside the determinism contract.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fetched_at: Option<String>,
    /// SHA-1 the publishing metadata declared for these bytes.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sha1_head: Option<String>,
    /// Where the metadata document that named this container was
    /// cached, relative to the VFS root.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ver_xml_cached: Option<String>,
    /// Minimum firmware version the metadata declared for this
    /// artifact.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub min_system_ver: Option<String>,
}

impl SourceRecord {
    /// A record for a container the operator supplied by hand: no
    /// acquisition provenance.
    #[must_use]
    pub fn local(kind: &str, sha256: HexSha256) -> Self {
        Self {
            kind: kind.to_string(),
            sha256,
            url: None,
            fetched_at: None,
            sha1_head: None,
            ver_xml_cached: None,
            min_system_ver: None,
        }
    }
}

/// The RAP installed for an NPDRM title, recorded so uninstall can
/// locate and verify it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RapRecord {
    /// RAP filename under the exdata directory (the full NPD content
    /// id plus `.rap`).
    pub filename: String,
    /// SHA-256 over the installed RAP bytes.
    pub sha256: HexSha256,
}

/// Title identity for an install record, all PARAM.SFO-derived.
///
/// `deny_unknown_fields` for the reason the wire shape carries it:
/// `system_ver` has a `default`, so without the gate a misspelling of
/// that key reads as a table that declared no `PS3_SYSTEM_VER`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TitleRecord {
    /// PARAM.SFO `TITLE_ID` (store-directory key).
    pub title_id: String,
    /// Full NPD content id (RAP-filename key) or the title-id.
    pub content_id: String,
    /// PARAM.SFO `CATEGORY`.
    pub category: String,
    /// PARAM.SFO `TITLE`.
    pub title: String,
    /// Install distribution tag (`psn-hdd` / `disc-iso`).
    pub distribution: String,
    /// PARAM.SFO `PS3_SYSTEM_VER`, spelled as the table spells it
    /// (`03.4000`): the lowest system software the title says it runs
    /// on. Distinct from [`SourceRecord::min_system_ver`], the
    /// publisher's claim.
    ///
    /// `None`, which is no claim about the title, when:
    ///
    /// - the table carries no such key, or an empty one;
    /// - the installer that wrote the record predates the field.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub system_ver: Option<String>,
}

/// A store entry's record: enough to verify a reinstall reproduces the
/// same tree from the same source, and to find that tree again.
///
/// Only [`Self::parse`] runs the gate. A record built field by field
/// never met it. Before a caller hands such a `store_path` to
/// [`StoreLayout::resolve_store_path`], it must satisfy that function's
/// precondition.
///
/// [`StoreLayout::resolve_store_path`]: crate::store::layout::StoreLayout::resolve_store_path
#[derive(Debug, Clone, Serialize)]
pub struct InstallRecord {
    /// Schema version.
    pub format_version: u32,
    /// Which store entry this record describes.
    pub artifact: ArtifactRecord,
    /// Source container provenance.
    pub source: SourceRecord,
    /// Title identity; absent for a firmware entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<TitleRecord>,
    /// Per-file SHA-256, keyed by tree-relative path. Empty for a
    /// firmware entry, whose per-file manifest is the `firmware.toml`
    /// inside the tree.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub files: BTreeMap<String, HexSha256>,
    /// The installed RAP, when the title is NPDRM with a network/local
    /// license. Absent (and omitted from the TOML) for disc, free, and
    /// no-RAP titles.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rap: Option<RapRecord>,
}

/// The wire shape, with `deny_unknown_fields` because the optional
/// blocks all carry a `default`: a misspelled `rap` would otherwise read
/// as a record with no RAP.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawInstallRecord {
    format_version: u32,
    artifact: ArtifactRecord,
    source: SourceRecord,
    #[serde(default)]
    title: Option<TitleRecord>,
    #[serde(default)]
    files: BTreeMap<String, HexSha256>,
    #[serde(default)]
    rap: Option<RapRecord>,
}

/// The schema stamp alone, read before the full shape.
///
/// A record from an older schema lacks blocks this one requires.
/// Without the stamp, [`InstallRecord::parse`] reports a missing field
/// instead of the version that explains it.
#[derive(Deserialize)]
struct FormatStamp {
    format_version: u32,
}

impl TryFrom<RawInstallRecord> for InstallRecord {
    type Error = InstallRecordParseError;

    fn try_from(raw: RawInstallRecord) -> Result<Self, Self::Error> {
        if raw.format_version != INSTALL_RECORD_FORMAT_VERSION {
            return Err(InstallRecordParseError::UnsupportedFormatVersion {
                found: raw.format_version,
                supported: INSTALL_RECORD_FORMAT_VERSION,
            });
        }
        let kind = raw.artifact.kind;
        if matches!(kind, ArtifactKind::Firmware | ArtifactKind::TitleUpdate)
            && !is_safe_component(&raw.artifact.version)
        {
            return Err(InstallRecordParseError::UnsafeArtifactVersion {
                kind,
                version: raw.artifact.version,
            });
        }
        if !store_path_is_safe(&raw.artifact.store_path) {
            return Err(InstallRecordParseError::UnsafeStorePath {
                path: raw.artifact.store_path,
            });
        }
        match kind {
            ArtifactKind::Firmware => {
                if raw.title.is_some() {
                    return Err(InstallRecordParseError::UnexpectedTitleBlock);
                }
                if raw.rap.is_some() {
                    return Err(InstallRecordParseError::UnexpectedRapBlock);
                }
                if !raw.files.is_empty() {
                    return Err(InstallRecordParseError::UnexpectedFilesBlock);
                }
            }
            ArtifactKind::TitleBase | ArtifactKind::TitleUpdate => {
                if raw.title.is_none() {
                    return Err(InstallRecordParseError::MissingTitleBlock { kind });
                }
            }
        }
        for path in raw.files.keys() {
            if !tree_rel_path_is_safe(path) {
                return Err(InstallRecordParseError::UnsafeFilePath { path: path.clone() });
            }
        }
        if let Some(rap) = &raw.rap {
            if !is_safe_component(&rap.filename) {
                return Err(InstallRecordParseError::UnsafeRapFilename {
                    filename: rap.filename.clone(),
                });
            }
        }
        if let Some(title) = &raw.title {
            for (field, value) in [
                ("title_id", &title.title_id),
                ("content_id", &title.content_id),
            ] {
                if !is_safe_component(value) {
                    return Err(InstallRecordParseError::UnsafeTitleKey {
                        field,
                        value: value.clone(),
                    });
                }
            }
        }
        Ok(Self {
            format_version: raw.format_version,
            artifact: raw.artifact,
            source: raw.source,
            title: raw.title,
            files: raw.files,
            rap: raw.rap,
        })
    }
}

/// Whether a `[files]` key stays inside the tree it is joined onto.
///
/// Looser than [`is_safe_component`]: container entry names carry spaces
/// and non-ASCII. `\` and `:` are refused because Win32 reads them as a
/// separator and a drive/stream marker where a POSIX host reads filename
/// characters, and an empty segment because it would make `a//b` and
/// `a/b` two keys for one file.
pub(crate) fn tree_rel_path_is_safe(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && !path.contains(':')
        && path
            .split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

impl InstallRecord {
    /// Parse a record, refusing one this build does not read.
    ///
    /// # Errors
    ///
    /// Every [`InstallRecordParseError`]: the schema gate, the
    /// block-shape checks, and the path/version safety checks all run
    /// here.
    pub fn parse(text: &str) -> Result<Self, InstallRecordParseError> {
        let stamp: FormatStamp = toml::from_str(text)?;
        if stamp.format_version != INSTALL_RECORD_FORMAT_VERSION {
            return Err(InstallRecordParseError::UnsupportedFormatVersion {
                found: stamp.format_version,
                supported: INSTALL_RECORD_FORMAT_VERSION,
            });
        }
        Self::try_from(toml::from_str::<RawInstallRecord>(text)?)
    }

    /// Serialize a record for writing beside the tree it describes.
    ///
    /// # Errors
    ///
    /// [`toml::ser::Error`] when a field cannot be represented.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string(self)
    }
}

#[cfg(test)]
#[path = "tests/record_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/title_block_gate_tests.rs"]
mod title_block_gate_tests;
