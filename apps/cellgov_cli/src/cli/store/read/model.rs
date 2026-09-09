//! The documents `--format json` emits, and the store paths they name.
//!
//! These documents are an API: field names are stable, and every
//! document carries [`STORE_FORMAT_VERSION`].

use std::path::Path;

use serde::Serialize;

/// Schema version every read command's JSON document carries.
///
/// Bump it only for a change a reader of the previous version could not
/// survive; a new field is additive and leaves it alone.
pub(crate) const STORE_FORMAT_VERSION: u32 = 2;

/// A path as a document names it: relative to the store root and
/// `/`-separated, the form an install record's `store_path` takes.
///
/// A path outside the root keeps its own spelling, because rendering it
/// relative would name a different file.
pub(crate) fn store_rel(root: &Path, path: &Path) -> String {
    let Ok(rel) = path.strip_prefix(root) else {
        return path.display().to_string();
    };
    let mut parts = Vec::new();
    for component in rel.components() {
        match component {
            std::path::Component::Normal(part) => parts.push(part.to_string_lossy()),
            // A component that is not a plain name (`..`, a root, a
            // Win32 prefix) cannot be dropped: the joined remainder
            // would name a different file than the path does.
            _ => return path.display().to_string(),
        }
    }
    parts.join("/")
}

/// One installed firmware version.
#[derive(Debug, Serialize)]
pub(crate) struct FirmwareDoc {
    /// The version key the entry is filed under.
    pub version: String,
    /// The directory the store files this version under.
    pub entry_dir: String,
    /// The install record describing it, absent when the version key is
    /// not a store directory name and so names no record path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<String>,
    /// SHA-256 over the PUP it was installed from.
    pub pup_sha256: String,
    /// PUP-header `image_version`, from the tree's `firmware.toml`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_version: Option<String>,
    /// Modules the tree's `firmware.toml` covers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modules: Option<usize>,
    /// Why the tree's `firmware.toml` did not load, when it did not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_error: Option<String>,
}

/// What a human report prints for a base whose PARAM.SFO named no
/// version. A boot summary's game identity prints the same words for
/// that tree.
pub(crate) const NO_VERSION_KEY: &str = "no version key";

/// A version as a human report prints it: under the key that named
/// it, the way a boot summary's game identity prints one.
///
/// An empty version prints [`NO_VERSION_KEY`] so it does not read as a
/// blank cell.
fn version_label(version: &str, key: Option<&str>) -> String {
    match key {
        _ if version.is_empty() => NO_VERSION_KEY.to_string(),
        Some(key) => format!("{key} {version}"),
        None => version.to_string(),
    }
}

/// A title's base install.
#[derive(Debug, Serialize)]
pub(crate) struct BaseDoc {
    /// The version the record names the base by, verbatim: PARAM.SFO
    /// `APP_VER`, or `VERSION` when the tree's table carries no
    /// `APP_VER`. A base install accepts a table that names neither and
    /// records the empty string, so this can be `""`.
    pub version: String,
    /// Which PARAM.SFO key named [`Self::version`], spelled as a boot
    /// summary's game identity spells it: `app_ver` or `sfo_version`.
    ///
    /// The key is absent when:
    ///
    /// - the table names no version;
    /// - the tree's table did not confirm the record
    ///   ([`Self::param_sfo_error`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_key: Option<String>,
    /// Why the tree's PARAM.SFO did not confirm [`Self::version`], when
    /// it did not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param_sfo_error: Option<String>,
    /// The install tree, as a store path.
    pub dir: String,
    /// Which tree the base holds: `game` or `disc`.
    pub tree: String,
    /// Install distribution tag.
    pub distribution: String,
    /// SHA-256 over the container it was installed from.
    pub source_sha256: String,
    /// PARAM.SFO `PS3_SYSTEM_VER` as the record holds it: the lowest
    /// system software the title says it runs on. Absent when the table
    /// declared none or the record predates the field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_ver: Option<String>,
    /// Version key of the system software the disc shipped in
    /// `PS3_UPDATE/`, which the disc install registered as a firmware
    /// entry. Absent under the conditions [`BaseEntry::shipped_firmware`]
    /// lists.
    ///
    /// [`BaseEntry::shipped_firmware`]: crate::composition::inventory::BaseEntry::shipped_firmware
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shipped_firmware: Option<String>,
    /// The install record describing it, absent when the title id is
    /// not a store directory name and so names no record path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<String>,
}

impl BaseDoc {
    /// The version as a human report prints it; see [`version_label`].
    pub(crate) fn version_label(&self) -> String {
        version_label(&self.version, self.version_key.as_deref())
    }
}

/// One installed update version.
#[derive(Debug, Serialize)]
pub(crate) struct UpdateDoc {
    /// The store key, verbatim from the update PKG's PARAM.SFO:
    /// `APP_VER`, or `VERSION` when the table carries no `APP_VER`.
    pub version: String,
    /// Which PARAM.SFO key named [`Self::version`]; see
    /// [`BaseDoc::version_key`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_key: Option<String>,
    /// Why the tree's PARAM.SFO did not confirm [`Self::version`], when
    /// it did not; see [`BaseDoc::param_sfo_error`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param_sfo_error: Option<String>,
    /// The `dev_hdd0/game` tree this update installs.
    pub dir: String,
    /// SHA-256 over the update PKG.
    pub source_sha256: String,
    /// Lowest firmware the publishing metadata declared.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_system_ver: Option<String>,
    /// PARAM.SFO `PS3_SYSTEM_VER` as the record holds it; see
    /// [`BaseDoc::system_ver`]. Distinct from [`Self::min_system_ver`],
    /// the publisher's claim.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_ver: Option<String>,
    /// The install record describing it, absent when the title id or
    /// version key is not a store directory name and so names no record
    /// path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<String>,
}

impl UpdateDoc {
    /// The version as a human report prints it; see [`version_label`].
    pub(crate) fn version_label(&self) -> String {
        version_label(&self.version, self.version_key.as_deref())
    }
}

/// One cell of a title's declared matrix, and whether it has an anchor.
#[derive(Debug, Serialize)]
pub(crate) struct AnchorDoc {
    /// Firmware version key of the cell.
    pub fw: String,
    /// `base` or an update version key; absent for a title shipped
    /// inside the firmware.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_ver: Option<String>,
    /// What the registry declares the cell to show.
    pub expect: String,
    /// True on the cell the generated documents render: the firmware
    /// the title's own PARAM.SFO asks for, times its base install.
    /// False on every cell of a title shipped inside the firmware.
    pub reference: bool,
    /// Whether a `boot_summary.json` is committed for the cell.
    pub recorded: bool,
    /// Whether both the cell's firmware and its game version are
    /// installed, so the cell can be booted here.
    pub installed: bool,
}

/// Everything the store and the registry hold about one title.
#[derive(Debug, Serialize)]
pub(crate) struct TitleDoc {
    /// The store key, and the guest directory name.
    pub title_id: String,
    /// Registry short name, absent when no `title_manifests/*.toml`
    /// names this title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_name: Option<String>,
    /// Registry display name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// The base install; absent for a title whose updates outlived it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<BaseDoc>,
    /// Whether the registry declares this title as one shipped inside
    /// the firmware image, which has no game-version axis of its own.
    pub ships_in_firmware: bool,
    /// Installed updates, ascending by version key.
    pub updates: Vec<UpdateDoc>,
    /// The registry's declared cells for this title.
    pub anchors: Vec<AnchorDoc>,
}

/// `firmware list` and `firmware show`.
#[derive(Debug, Serialize)]
pub(crate) struct FirmwareListDoc {
    /// See [`STORE_FORMAT_VERSION`].
    pub format_version: u32,
    /// The store root the entries were read under.
    pub store: String,
    /// The installed versions, ascending.
    pub firmware: Vec<FirmwareDoc>,
}

/// `title list` and `title show`.
#[derive(Debug, Serialize)]
pub(crate) struct TitleListDoc {
    /// See [`STORE_FORMAT_VERSION`].
    pub format_version: u32,
    /// The store root the entries were read under.
    pub store: String,
    /// The installed titles, ascending by title id.
    pub titles: Vec<TitleDoc>,
}

/// `status`.
#[derive(Debug, Serialize)]
pub(crate) struct StatusDoc {
    /// See [`STORE_FORMAT_VERSION`].
    pub format_version: u32,
    /// The store root the entries were read under.
    pub store: String,
    /// Bytes the store holds, summed over its entry trees; a floor when
    /// [`Self::unreadable_paths`] is non-zero.
    pub store_bytes: u64,
    /// Paths under the store the size walk could not read.
    pub unreadable_paths: usize,
    /// The installed firmware versions, ascending.
    pub firmware: Vec<FirmwareDoc>,
    /// The installed titles, ascending by title id.
    pub titles: Vec<TitleDoc>,
}

/// One artefact a verification found did not match.
#[derive(Debug, Serialize)]
pub(crate) struct DivergenceDoc {
    /// The artefact, as a store path.
    pub path: String,
    /// `missing`, `modified`, or `no-image`.
    pub kind: String,
    /// The hash the record holds, where there is one to compare.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    /// The hash found on disk, absent when nothing could be hashed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub found: Option<String>,
    /// Why no image came out of the file, for a `no-image` divergence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// One entry a verification pass examined.
#[derive(Debug, Serialize)]
pub(crate) struct VerifiedEntryDoc {
    /// Which entry this was: a firmware version, `base`, or an update
    /// version key.
    pub entry: String,
    /// Recorded artefacts whose hash matched.
    pub matched: usize,
    /// The artefacts that did not, in record order.
    pub divergences: Vec<DivergenceDoc>,
}

/// `firmware verify` and `title verify`.
#[derive(Debug, Serialize)]
pub(crate) struct VerifyDoc {
    /// See [`STORE_FORMAT_VERSION`].
    pub format_version: u32,
    /// The store root the entries were read under.
    pub store: String,
    /// What was verified: a firmware version key, or a title id.
    pub subject: String,
    /// One report per entry the pass covered.
    pub entries: Vec<VerifiedEntryDoc>,
    /// Whether every entry matched.
    pub clean: bool,
}

impl VerifyDoc {
    /// Artefacts that did not match, over every entry.
    pub(crate) fn diverged(&self) -> usize {
        self.entries.iter().map(|e| e.divergences.len()).sum()
    }

    /// Artefacts that matched, over every entry.
    pub(crate) fn matched(&self) -> usize {
        self.entries.iter().map(|e| e.matched).sum()
    }
}

#[cfg(test)]
#[path = "tests/model_tests.rs"]
mod tests;
