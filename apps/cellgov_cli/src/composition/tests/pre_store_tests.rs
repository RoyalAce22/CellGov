//! The pre-store refusal at [`StoreInventory::read`], the entry point
//! every command that reads the store shares.

use super::*;
use crate::composition::test_support::SyntheticStore;

const SYNTHETIC_TITLE_ID: &str = "TEST00000";

fn refusal(store: &SyntheticStore) -> String {
    match StoreInventory::read(store.root()) {
        Err(InventoryError::PreStore(e)) => e.to_string(),
        other => panic!("expected a pre-store refusal, got {other:?}"),
    }
}

#[test]
fn the_pre_store_firmware_mount_refuses_the_read_and_names_the_reinstall() {
    let store = SyntheticStore::new("prestore_fw");
    store.add_pre_store_firmware_mount();
    let msg = refusal(&store);
    assert!(msg.contains("dev_flash"), "{msg}");
    assert!(msg.contains("cellgov firmware install"), "{msg}");
}

/// A flat record is invisible to the kind-scoped walk.
#[test]
fn a_flat_install_record_refuses_the_read_rather_than_reading_as_empty() {
    let store = SyntheticStore::new("prestore_record");
    store.add_flat_install_record(SYNTHETIC_TITLE_ID);
    let msg = refusal(&store);
    assert!(msg.contains(SYNTHETIC_TITLE_ID), "{msg}");
    assert!(msg.contains("cellgov title install"), "{msg}");
}

#[test]
fn a_store_holding_a_firmware_and_a_title_reads_without_a_refusal() {
    let store = SyntheticStore::new("prestore_clean");
    store.add_firmware("4.93", true);
    store.add_base(SYNTHETIC_TITLE_ID, "01.00", false);
    let inventory = StoreInventory::read(store.root()).expect("the store layout is not refused");
    assert_eq!(inventory.firmware_versions(), vec!["4.93".to_string()]);
    assert!(inventory.title(SYNTHETIC_TITLE_ID).is_some());
}
