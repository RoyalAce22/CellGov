use super::*;
use crate::group::storage::StorageChange;
use cellgov_ps3_abi::lv1::repository::{field, first_field};
use cellgov_ps3_abi::lv1::storage_registration::{registration, StorageDevice};
use cellgov_time::GuestTicks;

/// A read in lpar 1 of `bus.<bus>.<n2>.<n3>`.
fn read(bus: u64, n2: u64, n3: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(0),
        number: 91,
        inputs: [1, first_field(b"bus", bus), n2, n3, 0, 0, 0, 0],
        pc: 0x8000_0000_0000_ce3c,
        lr: 0,
        thread: Some(1),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

fn registered(device: u64) -> StorageState {
    let mut state = StorageState::default();
    state.apply(StorageChange::DeviceRegistered(device));
    state
}

/// The device `id` of the 4.93 bus table.
fn device_4_93(id: u64) -> &'static StorageDevice {
    registration("4.93")
        .and_then(|source| source.device(id))
        .expect("a device of the 4.93 table")
}

/// The successful answer with `v1` in r4 and `v2` in r5.
fn answered(v1: u64, v2: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::success([v1, v2]))
}

#[test]
fn a_device_node_before_its_registration_is_the_unsourced_refusal() {
    let r = read(4, field(b"dev", 3), field(b"port", 0));
    let fresh = StorageState::default();
    assert_eq!(
        read_node_after_registrations(&r, Some("4.93"), &fresh),
        read_node(&r, Some("4.93"))
    );
    assert!(matches!(
        read_node(&r, Some("4.93")),
        HypercallDisposition::Refused(HypercallRefusal::RepositoryNode { .. })
    ));
}

#[test]
fn a_device_node_after_its_registration_answers_the_captured_values() {
    let r = read(4, field(b"dev", 3), field(b"port", 0));
    let state = registered(3);
    assert_eq!(
        read_node_after_registrations(&r, Some("4.93"), &state),
        answered(device_4_93(3).port, 0)
    );
    let num_dev = read(4, field(b"num_dev", 0), 0);
    let source = registration("4.93").expect("a 4.93 registration source");
    assert_eq!(
        read_node_after_registrations(&num_dev, Some("4.93"), &state),
        answered(source.num_dev, 0)
    );
}

#[test]
fn the_hypercall_91_handler_answers_a_node_that_a_registration_wrote() {
    let r = read(4, field(b"dev", 4), field(b"type", 0));
    let mut groups = crate::group::state::GroupStates::default();
    let memory = cellgov_mem::GuestMemory::new(0x1000);
    assert!(matches!(
        crate::handlers::answer(&r, Some("4.93"), &groups, &memory),
        HypercallDisposition::Refused(HypercallRefusal::RepositoryNode { .. })
    ));
    groups.apply(StorageChange::DeviceRegistered(4).into());
    assert_eq!(
        crate::handlers::answer(&r, Some("4.93"), &groups, &memory),
        answered(device_4_93(4).kind, 0)
    );
}

#[test]
fn a_table_node_and_an_absent_key_keep_their_answers_after_a_registration() {
    let state = registered(3);
    let bus_type = read(4, field(b"type", 0), 0);
    let absent = read(1, field(b"dev", 1), field(b"intr", 2));
    for r in [bus_type, absent] {
        assert_eq!(
            read_node_after_registrations(&r, Some("4.93"), &state),
            read_node(&r, Some("4.93"))
        );
        assert!(matches!(
            read_node(&r, Some("4.93")),
            HypercallDisposition::Answered(_)
        ));
    }
}
