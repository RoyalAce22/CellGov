use super::*;
use crate::composition::test_support::SyntheticStore;

const DISABLE_ENV: &str = "CELLGOV_NO_FIRMWARE_DIR";

fn inventory(store: &SyntheticStore) -> StoreInventory {
    StoreInventory::read(store.root()).unwrap()
}

#[test]
fn one_installed_firmware_is_selected_without_the_flag() {
    let store = SyntheticStore::new("sel_fw_one");
    store.add_firmware("4.93", true);
    let selected = select_firmware(&inventory(&store), None, None, DISABLE_ENV).unwrap();
    assert_eq!(selected.entry.version, "4.93");
    assert_eq!(selected.selected_by, FirmwareSelectedBy::Sole);
}

#[test]
fn no_installed_firmware_names_the_install_command_and_the_opt_out() {
    let store = SyntheticStore::new("sel_fw_none");
    let err = select_firmware(&inventory(&store), None, None, DISABLE_ENV).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("no firmware is installed"), "got: {msg}");
    assert!(msg.contains(DISABLE_ENV), "got: {msg}");
}

#[test]
fn several_installed_firmwares_refuse_and_list_them() {
    let store = SyntheticStore::new("sel_fw_many");
    store.add_firmware("3.55", true);
    store.add_firmware("4.91", true);
    let err = select_firmware(&inventory(&store), None, None, DISABLE_ENV).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("3.55, 4.91"), "got: {msg}");
    assert!(msg.contains("--fw"), "got: {msg}");
}

#[test]
fn the_flag_picks_one_of_several() {
    let store = SyntheticStore::new("sel_fw_pick");
    store.add_firmware("3.55", true);
    store.add_firmware("4.91", true);
    let selected = select_firmware(&inventory(&store), Some("3.55"), None, DISABLE_ENV).unwrap();
    assert_eq!(selected.entry.version, "3.55");
    assert_eq!(selected.selected_by, FirmwareSelectedBy::Flag);
}

#[test]
fn fw_naming_an_uninstalled_version_lists_the_installed_ones() {
    let store = SyntheticStore::new("sel_fw_miss");
    store.add_firmware("4.91", true);
    let err = select_firmware(&inventory(&store), Some("4.93"), None, DISABLE_ENV).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("\"4.93\" is not installed"), "got: {msg}");
    assert!(msg.contains("installed: 4.91"), "got: {msg}");
}

#[test]
fn a_recorded_firmware_whose_tree_is_gone_is_refused() {
    let store = SyntheticStore::new("sel_fw_gone");
    store.add_firmware("4.93", false);
    let err = select_firmware(&inventory(&store), None, None, DISABLE_ENV).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("4.93 is recorded"), "got: {msg}");
    assert!(msg.contains("is missing"), "got: {msg}");
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
fn a_base_plus_an_update_refuses_without_the_flag() {
    let store = SyntheticStore::new("sel_gv_many");
    store.add_base("NPAA00001", "01.00", false);
    store.add_update("NPAA00001", "02.51");
    let inv = inventory(&store);
    let err = select_game_version(inv.title("NPAA00001").unwrap(), None).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("base, 02.51"), "got: {msg}");
    assert!(msg.contains("--game-ver"), "got: {msg}");
}

#[test]
fn the_flag_selects_the_base_or_one_update() {
    let store = SyntheticStore::new("sel_gv_pick");
    store.add_base("NPAA00001", "01.00", false);
    store.add_update("NPAA00001", "02.51");
    let inv = inventory(&store);
    let entry = inv.title("NPAA00001").unwrap();
    assert_eq!(
        select_game_version(entry, Some("base")).unwrap(),
        GameVersion::Base
    );
    assert_eq!(
        select_game_version(entry, Some("02.51")).unwrap(),
        GameVersion::Update("02.51".to_string())
    );
}

#[test]
fn game_ver_naming_an_uninstalled_version_lists_the_installed_ones() {
    let store = SyntheticStore::new("sel_gv_miss");
    store.add_base("NPAA00001", "01.00", false);
    store.add_update("NPAA00001", "02.51");
    let inv = inventory(&store);
    let err = select_game_version(inv.title("NPAA00001").unwrap(), Some("03.00")).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("\"03.00\" is not installed"), "got: {msg}");
    assert!(msg.contains("base, 02.51"), "got: {msg}");
}

#[test]
fn an_orphan_update_refuses_whether_or_not_the_flag_names_it() {
    let store = SyntheticStore::new("sel_gv_orphan");
    store.add_update("NPAA00001", "02.51");
    let inv = inventory(&store);
    let entry = inv.title("NPAA00001").unwrap();
    for asked in [None, Some("02.51")] {
        let msg = select_game_version(entry, asked).unwrap_err().to_string();
        assert!(msg.contains("no base"), "got: {msg}");
        assert!(msg.contains("02.51"), "got: {msg}");
    }
}

#[test]
fn version_strings_are_compared_verbatim() {
    let store = SyntheticStore::new("sel_gv_verbatim");
    store.add_base("NPAA00001", "01.00", false);
    store.add_update("NPAA00001", "02.51");
    let inv = inventory(&store);
    let entry = inv.title("NPAA00001").unwrap();
    let err = select_game_version(entry, Some("2.51")).unwrap_err();
    assert!(
        matches!(err, GameVersionSelectError::NotInstalled { .. }),
        "got: {err}"
    );
    let msg = err.to_string();
    assert!(msg.contains("\"2.51\" is not installed"), "got: {msg}");
    assert!(msg.contains("base, 02.51"), "got: {msg}");
}
