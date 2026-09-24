//! Builds the read commands' documents from the store's records and the
//! title registry.
//!
//! Every read command renders the same documents.

use std::path::{Path, PathBuf};

use cellgov_install::store::{Artifact, StoreLayout, TitleId, VersionKey};

use crate::composition::identity::tree_app_version;
use cellgov_boot::manifest::{CellKey, GameSource, TitleManifest, TitleRegistry, BASE_GAME_VER};
use cellgov_install::store::inventory::{FirmwareEntry, StoreInventory, TitleEntry};

use super::model::{
    store_rel, AnchorDoc, BaseDoc, CoreOsDoc, CoreOsFileDoc, FirmwareDoc, KernelDoc, TitleDoc,
    UpdateDoc, STORE_FORMAT_VERSION,
};

/// The store root, the inventory read under it, and the registry the
/// cells come from.
pub(crate) struct StoreView {
    /// The store root every path is rendered relative to.
    pub root: PathBuf,
    /// What the install records say is installed.
    pub inventory: StoreInventory,
    /// The declared titles, for short names and cell matrices.
    pub registry: TitleRegistry,
    /// Where the view looks for committed anchors.
    pub fixtures: PathBuf,
}

impl StoreView {
    /// See [`STORE_FORMAT_VERSION`].
    pub(crate) fn format_version(&self) -> u32 {
        STORE_FORMAT_VERSION
    }

    /// The store root, as a document names it.
    pub(crate) fn store_label(&self) -> String {
        self.root.display().to_string()
    }

    /// A path, relative to this store root.
    pub(crate) fn rel(&self, path: &Path) -> String {
        store_rel(&self.root, path)
    }

    /// The record path for one `Artifact`, relative to the store root.
    fn record_rel(&self, artifact: &Artifact) -> String {
        self.rel(&StoreLayout::new(&self.root).record_path(artifact))
    }

    /// One firmware entry as a document.
    ///
    /// `image_version` and `modules` come from the `firmware.toml`
    /// inside the tree. A tree with no readable `firmware.toml` leaves
    /// both absent and names why in `manifest_error`.
    pub(crate) fn firmware_doc(&self, entry: &FirmwareEntry) -> FirmwareDoc {
        let (manifest, manifest_error) =
            match cellgov_install::firmware_verify::load_manifest(&entry.dev_flash_dir()) {
                Ok(manifest) => (Some(manifest), None),
                Err(e) => (None, Some(e.to_string())),
            };
        // The record gate refuses a firmware version that is not a store
        // directory name, so this key is one.
        let record = VersionKey::new(&entry.version)
            .ok()
            .map(|version| self.record_rel(&Artifact::Firmware { version }));
        FirmwareDoc {
            version: entry.version.clone(),
            entry_dir: self.rel(&entry.entry_dir),
            record,
            pup_sha256: entry.pup_sha256.clone(),
            image_version: manifest.as_ref().map(|m| m.firmware.image_version.clone()),
            modules: manifest.as_ref().map(|m| m.files.len()),
            manifest_error,
            core_os: entry.core_os.as_ref().map(|block| CoreOsDoc {
                kernel: block.kernel.as_ref().map(|k| KernelDoc {
                    path: k.path.clone(),
                    stored_sha256: k.stored_sha256.to_hex(),
                }),
                omission: block.omission.clone(),
                files: block
                    .files
                    .iter()
                    .map(|f| CoreOsFileDoc {
                        name: f.name.clone(),
                        size: f.size,
                    })
                    .collect(),
            }),
        }
    }

    /// Every installed firmware version as documents, ascending.
    pub(crate) fn firmware_docs(&self) -> Vec<FirmwareDoc> {
        self.inventory
            .firmware_entries()
            .map(|e| self.firmware_doc(e))
            .collect()
    }

    /// One title as a document, with its registry identity and cells.
    ///
    /// The view reads each installed tree's PARAM.SFO for the key that
    /// named its version: one file read per base and per update.
    pub(crate) fn title_doc(&self, entry: &TitleEntry) -> TitleDoc {
        let manifest = self.registry.by_content_id(&entry.title_id);
        let key = TitleId::new(&entry.title_id).ok();
        let base = entry.base.as_ref().map(|base| {
            let (version_key, param_sfo_error) = version_key(base.param_sfo_path(), &base.version);
            BaseDoc {
                version: base.version.clone(),
                version_key,
                param_sfo_error,
                dir: self.rel(&base.dir),
                tree: base.tree.dir_name().to_string(),
                distribution: base.distribution.clone(),
                source_sha256: base.source_sha256.clone(),
                system_ver: base.system_ver.clone(),
                shipped_firmware: base.shipped_firmware.clone(),
                record: key
                    .clone()
                    .map(|title_id| self.record_rel(&Artifact::TitleBase { title_id })),
            }
        });
        let updates = entry
            .updates
            .values()
            .map(|update| {
                let (version_key, param_sfo_error) =
                    version_key(update.param_sfo_path(), &update.version);
                UpdateDoc {
                    version: update.version.clone(),
                    version_key,
                    param_sfo_error,
                    dir: self.rel(&update.dir),
                    source_sha256: update.source_sha256.clone(),
                    min_system_ver: update.min_system_ver.clone(),
                    system_ver: update.system_ver.clone(),
                    record: match (key.clone(), VersionKey::new(&update.version)) {
                        (Some(title_id), Ok(version)) => {
                            Some(self.record_rel(&Artifact::TitleUpdate { title_id, version }))
                        }
                        // The store filed this entry under both keys, so it
                        // accepted both as directory names.
                        _ => None,
                    },
                }
            })
            .collect();
        TitleDoc {
            title_id: entry.title_id.clone(),
            short_name: manifest.map(|m| m.short_name.clone()),
            display_name: manifest.map(|m| m.display_name.clone()),
            ships_in_firmware: manifest.is_some_and(ships_in_firmware),
            base,
            updates,
            anchors: manifest.map_or_else(Vec::new, |m| self.anchor_docs(m, Some(entry))),
        }
    }

    /// Every installed title as documents, ascending by title id.
    pub(crate) fn title_docs(&self) -> Vec<TitleDoc> {
        self.inventory.titles().map(|t| self.title_doc(t)).collect()
    }

    /// A declared title with no store entry, so `status` can name a
    /// registry title nothing is installed for.
    pub(crate) fn declared_only_doc(&self, manifest: &TitleManifest) -> TitleDoc {
        TitleDoc {
            title_id: manifest.content_id.clone(),
            short_name: Some(manifest.short_name.clone()),
            display_name: Some(manifest.display_name.clone()),
            ships_in_firmware: ships_in_firmware(manifest),
            base: None,
            updates: Vec::new(),
            anchors: self.anchor_docs(manifest, None),
        }
    }

    /// The registry's declared cells for `manifest`, each marked with
    /// whether an anchor is committed and whether this machine holds the
    /// versions the cell names.
    fn anchor_docs(&self, manifest: &TitleManifest, entry: Option<&TitleEntry>) -> Vec<AnchorDoc> {
        let reference = manifest.reference_key();
        manifest
            .matrix
            .iter()
            .map(|cell| AnchorDoc {
                fw: cell.key.fw.clone(),
                game_ver: cell.key.game_ver.clone(),
                expect: cell.expect.label().to_string(),
                reference: reference.as_ref() == Some(&cell.key),
                recorded: crate::paths::boot_anchor_path_in(
                    &self.fixtures,
                    &manifest.content_id,
                    &cell.key,
                )
                .is_file(),
                installed: self.cell_is_installed(&cell.key, entry),
            })
            .collect()
    }

    /// Whether this machine holds both halves of a cell.
    ///
    /// A title shipped inside the firmware has no game-version axis, so
    /// its firmware entry is the whole answer.
    fn cell_is_installed(&self, cell: &CellKey, entry: Option<&TitleEntry>) -> bool {
        if self.inventory.firmware(&cell.fw).is_none() {
            return false;
        }
        match (&cell.game_ver, entry) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(v), Some(entry)) => game_version_is_installed(v, entry),
        }
    }
}

/// The `version_key` and `param_sfo_error` pair of a base or update
/// document. The first names the PARAM.SFO key that named `recorded`;
/// the second says why the tree's table did not confirm it.
fn version_key(param_sfo: PathBuf, recorded: &str) -> (Option<String>, Option<String>) {
    match tree_app_version(param_sfo, recorded) {
        Ok(found) => (found.map(|v| v.key().to_string()), None),
        Err(e) => (None, Some(e.to_string())),
    }
}

/// Whether one title entry holds the game version a cell names, in a
/// form that composes into a boot.
///
/// An update tree patches a base and cannot be composed alone
/// (`crate::composition::select::select_game_version`). An update
/// archived over an uninstalled base is not a version this machine can
/// boot.
fn game_version_is_installed(game_ver: &str, entry: &TitleEntry) -> bool {
    if entry.base.is_none() {
        return false;
    }
    game_ver == BASE_GAME_VER || entry.updates.contains_key(game_ver)
}

/// Whether the registry declares `manifest` as a title shipped inside
/// the firmware image.
fn ships_in_firmware(manifest: &TitleManifest) -> bool {
    matches!(manifest.source, GameSource::FirmwareExec { .. })
}

#[cfg(test)]
#[path = "tests/collect_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/version_key_tests.rs"]
mod version_key_tests;
