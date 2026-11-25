use super::*;
use cellgov_ps3_abi::lv1::partition::LPAR_ID_PME;
use cellgov_ps3_abi::lv1::repository::{field, first_field};
use cellgov_ps3_abi::lv1::storage_registration::PLACEHOLDER_V2;

/// The key `bus.4.dev.<device>.<name>` of lpar 1.
fn device_key(device: u64, name: &[u8]) -> RepositoryKey {
    RepositoryKey {
        lpar: LPAR_ID_PME,
        n1: first_field(b"bus", 4),
        n2: field(b"dev", device),
        n3: field(name, 0),
        n4: 0,
    }
}

fn registered(devices: &[u64]) -> StorageState {
    let mut state = StorageState::default();
    for &device in devices {
        state.apply(register_device(Some("4.93"), device).expect("a table device"));
    }
    state
}

#[test]
fn each_4_93_table_device_registers_under_its_own_id() {
    for device in 0..5 {
        assert_eq!(
            register_device(Some("4.93"), device),
            Ok(StorageChange::DeviceRegistered(device))
        );
    }
}

#[test]
fn a_device_outside_the_captured_table_is_refused_by_name() {
    for device in [5, 6, 7, u64::MAX] {
        assert_eq!(register_device(Some("4.93"), device), Err(UNKNOWN_DEVICE));
    }
}

#[test]
fn a_firmware_with_no_registration_source_is_refused_by_name() {
    assert_eq!(register_device(None, 0), Err(UNSOURCED_FIRMWARE));
    assert_eq!(register_device(Some("3.70"), 0), Err(UNSOURCED_FIRMWARE));
}

#[test]
fn no_node_answers_before_lv1_registers_a_device() {
    let state = StorageState::default();
    assert_eq!(
        registered_node(&state, Some("4.93"), &device_key(3, b"port")),
        None
    );
}

#[test]
fn a_registration_answers_its_device_and_the_placeholders_of_the_others() {
    let device_3 = registration("4.93")
        .and_then(|source| source.device(3))
        .expect("device 3 of the 4.93 table");
    let state = registered(&[3]);
    let at = |device, name: &[u8]| registered_node(&state, Some("4.93"), &device_key(device, name));
    assert_eq!(at(3, b"port"), Some((device_3.port, 0)));
    assert_eq!(at(3, b"blk_size"), Some((device_3.blk_size, 0)));
    assert_eq!(at(2, b"blk_size"), Some((0, PLACEHOLDER_V2)));
}

#[test]
fn a_registered_node_at_a_firmware_with_no_source_is_none() {
    let state = registered(&[3]);
    for firmware in [None, Some("3.70")] {
        assert_eq!(
            registered_node(&state, firmware, &device_key(3, b"port")),
            None
        );
    }
}
