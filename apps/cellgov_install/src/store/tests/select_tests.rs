use super::*;
use cellgov_testkit::store::SyntheticStore;

fn inventory(store: &SyntheticStore) -> StoreInventory {
    StoreInventory::read(store.root()).unwrap()
}

fn versions(list: &[&str]) -> Vec<String> {
    list.iter().map(ToString::to_string).collect()
}

#[test]
fn one_installed_firmware_is_selected_without_a_name() {
    let store = SyntheticStore::new("sel_fw_one");
    store.add_firmware("4.93", true);
    let selected = select_firmware(&inventory(&store), None, None).unwrap();
    assert_eq!(selected.entry.version, "4.93");
    assert_eq!(selected.selected_by, FirmwareSelectedBy::Sole);
}

#[test]
fn no_installed_firmware_is_refused_as_none_installed() {
    let store = SyntheticStore::new("sel_fw_none");
    let err = select_firmware(&inventory(&store), None, None).unwrap_err();
    assert!(
        matches!(err, FirmwareSelectError::NoneInstalled { .. }),
        "got: {err}"
    );
}

#[test]
fn several_installed_firmwares_refuse_and_list_them() {
    let store = SyntheticStore::new("sel_fw_many");
    store.add_firmware("3.55", true);
    store.add_firmware("4.91", true);
    let err = select_firmware(&inventory(&store), None, None).unwrap_err();
    match err {
        FirmwareSelectError::Ambiguous { installed, .. } => {
            assert_eq!(installed, versions(&["3.55", "4.91"]));
        }
        other => panic!("expected Ambiguous, got {other:?}"),
    }
}

#[test]
fn a_name_picks_one_of_several() {
    let store = SyntheticStore::new("sel_fw_pick");
    store.add_firmware("3.55", true);
    store.add_firmware("4.91", true);
    let selected = select_firmware(&inventory(&store), Some("3.55"), None).unwrap();
    assert_eq!(selected.entry.version, "3.55");
    assert_eq!(selected.selected_by, FirmwareSelectedBy::Named);
}

/// A shipped version is the default, never an override: a name wins,
/// and with no name the shipped version selects over the count.
#[test]
fn a_name_outranks_the_shipped_firmware_and_the_shipped_one_outranks_the_count() {
    let store = SyntheticStore::new("sel_fw_shipped");
    store.add_firmware("3.55", true);
    store.add_firmware("4.91", true);
    let inv = inventory(&store);
    let named = select_firmware(&inv, Some("4.91"), Some("3.55")).unwrap();
    assert_eq!(named.entry.version, "4.91");
    assert_eq!(named.selected_by, FirmwareSelectedBy::Named);
    let shipped = select_firmware(&inv, None, Some("3.55")).unwrap();
    assert_eq!(shipped.entry.version, "3.55");
    assert_eq!(shipped.selected_by, FirmwareSelectedBy::Shipped);
}

#[test]
fn naming_an_uninstalled_firmware_lists_the_installed_ones() {
    let store = SyntheticStore::new("sel_fw_miss");
    store.add_firmware("4.91", true);
    let err = select_firmware(&inventory(&store), Some("4.93"), None).unwrap_err();
    assert_eq!(
        err,
        FirmwareSelectError::NotInstalled {
            asked: "4.93".to_string(),
            root: store.root().display().to_string(),
            installed: versions(&["4.91"]),
        }
    );
}

#[test]
fn a_recorded_firmware_whose_tree_is_gone_is_refused() {
    let store = SyntheticStore::new("sel_fw_gone");
    store.add_firmware("4.93", false);
    let err = select_firmware(&inventory(&store), None, None).unwrap_err();
    assert_eq!(
        err,
        FirmwareSelectError::TreeMissing {
            version: "4.93".to_string(),
            root: store.root().display().to_string(),
            dir: store.firmware_dev_flash("4.93").display().to_string(),
        }
    );
}

#[test]
fn a_base_with_no_updates_is_the_single_candidate() {
    let store = SyntheticStore::new("sel_gv_one");
    store.add_base("NPAA00001", "01.00", false);
    let inv = inventory(&store);
    let entry = inv.title("NPAA00001").unwrap();
    assert_eq!(select_game_version(entry, None).unwrap(), GameVersion::Base);
}

#[test]
fn a_base_plus_an_update_refuses_without_a_name() {
    let store = SyntheticStore::new("sel_gv_many");
    store.add_base("NPAA00001", "01.00", false);
    store.add_update("NPAA00001", "02.51");
    let inv = inventory(&store);
    let err = select_game_version(inv.title("NPAA00001").unwrap(), None).unwrap_err();
    assert_eq!(
        err,
        GameVersionSelectError::Ambiguous {
            title_id: "NPAA00001".to_string(),
            installed: versions(&["base", "02.51"]),
        }
    );
}

#[test]
fn a_name_selects_the_base_or_one_update() {
    let store = SyntheticStore::new("sel_gv_pick");
    store.add_base("NPAA00001", "01.00", false);
    store.add_update("NPAA00001", "02.51");
    let inv = inventory(&store);
    let entry = inv.title("NPAA00001").unwrap();
    assert_eq!(
        select_game_version(entry, Some(BASE_GAME_VER)).unwrap(),
        GameVersion::Base
    );
    assert_eq!(
        select_game_version(entry, Some("02.51")).unwrap(),
        GameVersion::Update("02.51".to_string())
    );
}

#[test]
fn naming_an_uninstalled_version_lists_the_installed_ones() {
    let store = SyntheticStore::new("sel_gv_miss");
    store.add_base("NPAA00001", "01.00", false);
    store.add_update("NPAA00001", "02.51");
    let inv = inventory(&store);
    let err = select_game_version(inv.title("NPAA00001").unwrap(), Some("03.00")).unwrap_err();
    assert_eq!(
        err,
        GameVersionSelectError::NotInstalled {
            asked: "03.00".to_string(),
            title_id: "NPAA00001".to_string(),
            installed: versions(&["base", "02.51"]),
        }
    );
}

#[test]
fn an_orphan_update_refuses_whether_or_not_it_is_named() {
    let store = SyntheticStore::new("sel_gv_orphan");
    store.add_update("NPAA00001", "02.51");
    let inv = inventory(&store);
    let entry = inv.title("NPAA00001").unwrap();
    for asked in [None, Some("02.51")] {
        assert_eq!(
            select_game_version(entry, asked).unwrap_err(),
            GameVersionSelectError::OrphanUpdates {
                title_id: "NPAA00001".to_string(),
                updates: versions(&["02.51"]),
            }
        );
    }
}

#[test]
fn version_strings_are_compared_verbatim() {
    let store = SyntheticStore::new("sel_gv_verbatim");
    store.add_base("NPAA00001", "01.00", false);
    store.add_update("NPAA00001", "02.51");
    let inv = inventory(&store);
    let entry = inv.title("NPAA00001").unwrap();
    assert!(matches!(
        select_game_version(entry, Some("2.51")).unwrap_err(),
        GameVersionSelectError::NotInstalled { .. }
    ));
}

#[test]
fn an_empty_version_list_renders_as_none() {
    assert_eq!(render_list(&[]), "(none)");
    assert_eq!(render_list(&versions(&["base", "02.51"])), "base, 02.51");
}
