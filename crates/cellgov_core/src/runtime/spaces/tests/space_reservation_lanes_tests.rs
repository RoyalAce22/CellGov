//! Child-space reservation tables take lanes of their own in the
//! sync-state sum.

use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_sync::ReservedLine;

use super::*;
use crate::runtime::state::Runtime;

const S1: AddressSpaceId = AddressSpaceId::new(1);

fn with_child_space() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(16), cellgov_time::Budget::new(4), 100);
    rt.create_address_space(S1).unwrap();
    rt
}

#[test]
fn one_reservation_in_a_child_space_and_in_the_boot_space_hash_differently() {
    let line = ReservedLine::containing(0x80);
    let mut child = with_child_space();
    child
        .space_reservations_mut(S1)
        .unwrap()
        .insert_or_replace(UnitId::new(0), line);
    let mut boot = with_child_space();
    boot.space_reservations_mut(AddressSpaceId::BOOT)
        .unwrap()
        .insert_or_replace(UnitId::new(0), line);
    assert_ne!(child.sync_state_hash(), boot.sync_state_hash());
    for rt in [&child, &boot] {
        assert_eq!(rt.sync_state_hash(), rt.sync_state_hash_from_scratch());
    }
}
