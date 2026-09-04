use super::*;
use crate::composition::test_support::SyntheticStore;

#[test]
fn empty_store_reads_as_nothing_installed() {
    let store = SyntheticStore::new("inv_empty");
    let inventory = StoreInventory::read(store.root()).unwrap();
    assert!(inventory.firmware_versions().is_empty());
    assert!(inventory.title("NPAA00001").is_none());
}

#[test]
fn firmware_entry_dir_follows_the_record_store_path() {
    let store = SyntheticStore::new("inv_fw");
    store.add_firmware("4.93", true);
    let inventory = StoreInventory::read(store.root()).unwrap();
    let entry = inventory.firmware("4.93").unwrap();
    assert_eq!(entry.entry_dir, store.firmware_entry("4.93"));
    assert_eq!(entry.dev_flash_dir(), store.firmware_dev_flash("4.93"));
}

#[test]
fn disc_distribution_selects_the_disc_tree() {
    let store = SyntheticStore::new("inv_disc");
    store.add_base("BLAA00001", "02.00", true);
    let inventory = StoreInventory::read(store.root()).unwrap();
    let base = inventory.title("BLAA00001").unwrap().base.as_ref().unwrap();
    assert_eq!(base.tree, TitleTree::Disc);
    assert_eq!(base.app_ver, "02.00");
}

#[test]
fn updates_are_ordered_by_version_string() {
    let store = SyntheticStore::new("inv_updates");
    store.add_base("NPAA00001", "01.00", false);
    store.add_update("NPAA00001", "02.51");
    store.add_update("NPAA00001", "01.01");
    let inventory = StoreInventory::read(store.root()).unwrap();
    let entry = inventory.title("NPAA00001").unwrap();
    assert_eq!(
        entry.updates.keys().collect::<Vec<_>>(),
        vec!["01.01", "02.51"]
    );
    assert_eq!(entry.candidates(), vec!["base", "01.01", "02.51"]);
}

#[test]
fn an_update_without_a_base_reads_as_an_orphan() {
    let store = SyntheticStore::new("inv_orphan");
    store.add_update("NPAA00001", "01.01");
    let inventory = StoreInventory::read(store.root()).unwrap();
    let entry = inventory.title("NPAA00001").unwrap();
    assert!(entry.base.is_none());
    assert_eq!(entry.candidates(), vec!["01.01"]);
}

#[test]
fn a_record_this_build_does_not_read_is_named_rather_than_skipped() {
    let store = SyntheticStore::new("inv_v2");
    std::fs::create_dir_all(store.root().join(".cellgov/installs/firmware")).unwrap();
    std::fs::write(
        store
            .root()
            .join(".cellgov/installs/firmware/4.93.install.toml"),
        "format_version = 2\n\n[artifact]\nkind = \"firmware\"\nversion = \"4.93\"\n\
         store_path = \"firmware/4.93\"\n\n[source]\nkind = \"pup\"\nsha256 = \"00\"\n",
    )
    .unwrap();
    let err = StoreInventory::read(store.root()).unwrap_err();
    assert!(
        matches!(err, InventoryError::ParseRecord { .. }),
        "got: {err}"
    );
}

#[test]
fn a_record_filed_under_the_wrong_kind_is_refused() {
    let store = SyntheticStore::new("inv_kind");
    store.add_base("NPAA00001", "01.00", false);
    // A base record filed where an update record belongs.
    let dir = store.root().join(".cellgov/installs/titles/NPAA00001");
    let base = std::fs::read_to_string(dir.join("base.install.toml")).unwrap();
    std::fs::write(dir.join("update-01.01.install.toml"), base).unwrap();
    let err = StoreInventory::read(store.root()).unwrap_err();
    assert!(
        matches!(err, InventoryError::KindMismatch { .. }),
        "got: {err}"
    );
}

#[test]
fn a_record_naming_another_title_is_refused() {
    let store = SyntheticStore::new("inv_titleid");
    store.add_base("NPAA00001", "01.00", false);
    let dir = store.root().join(".cellgov/installs/titles/NPAA00001");
    let record = std::fs::read_to_string(dir.join("base.install.toml")).unwrap();
    std::fs::create_dir_all(store.root().join(".cellgov/installs/titles/BLAA00001")).unwrap();
    std::fs::write(
        store
            .root()
            .join(".cellgov/installs/titles/BLAA00001/base.install.toml"),
        record,
    )
    .unwrap();
    let err = StoreInventory::read(store.root()).unwrap_err();
    assert!(
        matches!(err, InventoryError::TitleIdMismatch { .. }),
        "got: {err}"
    );
}

#[test]
fn exdata_roots_list_the_live_directory_before_the_per_title_ones() {
    let store = SyntheticStore::new("inv_exdata");
    store.add_base("NPAA00001", "01.00", false);
    store.add_title_rap("NPAA00001", "a.rap", b"0123456789abcdef");
    let live = store.root().join("dev_hdd0/home/00000001/exdata");
    std::fs::create_dir_all(&live).unwrap();
    let inventory = StoreInventory::read(store.root()).unwrap();
    assert_eq!(
        inventory.exdata_roots().unwrap(),
        vec![live, store.root().join("titles/NPAA00001/exdata")]
    );
}

#[test]
fn an_update_entry_names_the_game_tree_not_the_entry_directory() {
    let store = SyntheticStore::new("inv_update_tree");
    store.add_base("NPAA00001", "01.00", false);
    store.add_update("NPAA00001", "02.51");
    let inventory = StoreInventory::read(store.root()).unwrap();
    let update = &inventory.title("NPAA00001").unwrap().updates["02.51"];
    assert_eq!(update.dir, store.update_tree("NPAA00001", "02.51"));
}

#[test]
fn a_second_record_claiming_one_version_is_refused_rather_than_replacing_the_first() {
    let store = SyntheticStore::new("inv_dup");
    store.add_firmware("4.93", true);
    let dir = store.root().join(".cellgov/installs/firmware");
    let record = std::fs::read_to_string(dir.join("4.93.install.toml")).unwrap();
    std::fs::write(dir.join("4.93-copy.install.toml"), record).unwrap();
    let err = StoreInventory::read(store.root()).unwrap_err();
    assert!(
        matches!(err, InventoryError::MisfiledRecord { .. }),
        "got: {err}"
    );
}

#[test]
fn an_update_record_filed_under_another_version_is_refused() {
    let store = SyntheticStore::new("inv_misfiled_update");
    store.add_base("NPAA00001", "01.00", false);
    store.add_update("NPAA00001", "02.51");
    let dir = store.root().join(".cellgov/installs/titles/NPAA00001");
    let record = std::fs::read_to_string(dir.join("update-02.51.install.toml")).unwrap();
    std::fs::remove_file(dir.join("update-02.51.install.toml")).unwrap();
    std::fs::write(dir.join("update-02.50.install.toml"), record).unwrap();
    let err = StoreInventory::read(store.root()).unwrap_err();
    assert!(
        matches!(err, InventoryError::MisfiledRecord { .. }),
        "got: {err}"
    );
}
