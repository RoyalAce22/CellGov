use super::*;

use std::collections::BTreeMap;

use cellgov_install::store::TitleTree;

use crate::composition::inventory::{BaseEntry, UpdateEntry};
use crate::game::manifest::TitleRegistry;

/// Placeholder identity: these cases build every entry by hand and name
/// no installed corpus.
const TITLE_ID: &str = "TEST00000";

/// A view over an empty store: each case builds the entries it needs.
fn view() -> StoreView {
    StoreView {
        root: PathBuf::from("vfs"),
        inventory: StoreInventory::read(Path::new("no-such-store-root"))
            .expect("an absent root reads as an empty store"),
        registry: TitleRegistry::default(),
        fixtures: PathBuf::from("no-such-fixtures"),
    }
}

fn title_entry(base: Option<BaseEntry>, updates: &[&str]) -> TitleEntry {
    TitleEntry {
        title_id: TITLE_ID.to_string(),
        base,
        updates: updates
            .iter()
            .map(|version| {
                (
                    (*version).to_string(),
                    UpdateEntry {
                        version: (*version).to_string(),
                        dir: PathBuf::from("vfs")
                            .join("titles")
                            .join(TITLE_ID)
                            .join("updates")
                            .join(version)
                            .join("game"),
                        source_sha256: "cd".repeat(32),
                        min_system_ver: Some("03.5000".to_string()),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>(),
        exdata_dir: PathBuf::from("vfs")
            .join("titles")
            .join(TITLE_ID)
            .join("exdata"),
    }
}

fn base_entry() -> BaseEntry {
    BaseEntry {
        app_ver: "01.00".to_string(),
        dir: PathBuf::from("vfs")
            .join("dev_hdd0")
            .join("game")
            .join(TITLE_ID),
        tree: TitleTree::Game,
        distribution: "psn-hdd".to_string(),
        source_sha256: "ab".repeat(32),
    }
}

#[test]
fn a_title_document_names_the_record_the_store_files_each_entry_under() {
    let doc = view().title_doc(&title_entry(Some(base_entry()), &["02.51"]));
    assert_eq!(
        doc.base.as_ref().and_then(|b| b.record.as_deref()),
        Some(".cellgov/installs/titles/TEST00000/base.install.toml")
    );
    assert_eq!(
        doc.updates[0].record.as_deref(),
        Some(".cellgov/installs/titles/TEST00000/update-02.51.install.toml")
    );
}

#[test]
fn every_path_a_document_names_is_relative_to_the_store_root() {
    let doc = view().title_doc(&title_entry(Some(base_entry()), &["02.51"]));
    assert_eq!(
        doc.base.as_ref().map(|b| b.dir.as_str()),
        Some("dev_hdd0/game/TEST00000")
    );
    assert_eq!(doc.updates[0].dir, "titles/TEST00000/updates/02.51/game");
}

#[test]
fn a_title_no_manifest_declares_carries_no_short_name_and_no_cells() {
    let doc = view().title_doc(&title_entry(Some(base_entry()), &[]));
    assert!(doc.short_name.is_none());
    assert!(doc.anchors.is_empty());
}

#[test]
fn an_update_carries_the_minimum_firmware_its_metadata_declared() {
    let doc = view().title_doc(&title_entry(Some(base_entry()), &["02.51"]));
    assert_eq!(doc.updates[0].min_system_ver.as_deref(), Some("03.5000"));
}

#[test]
fn an_update_archived_over_no_base_is_not_an_installed_version() {
    let entry = title_entry(None, &["02.51"]);
    assert!(!game_version_is_installed("02.51", &entry));
    assert!(!game_version_is_installed(BASE_GAME_VER, &entry));
}

#[test]
fn an_update_over_an_installed_base_is_an_installed_version() {
    let entry = title_entry(Some(base_entry()), &["02.51"]);
    assert!(game_version_is_installed("02.51", &entry));
    assert!(game_version_is_installed(BASE_GAME_VER, &entry));
    assert!(!game_version_is_installed("02.10", &entry));
}

#[test]
fn a_title_whose_base_is_gone_still_lists_its_updates() {
    let doc = view().title_doc(&title_entry(None, &["02.10", "02.51"]));
    assert!(doc.base.is_none());
    assert_eq!(doc.updates.len(), 2);
    assert_eq!(doc.updates[0].version, "02.10");
}
