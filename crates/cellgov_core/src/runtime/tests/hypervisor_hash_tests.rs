use cellgov_event::UnitId;
use cellgov_lv2::hypervisor::{HypercallDisposition, HypercallRequest};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::call::ALLOCATE_MEMORY;
use cellgov_time::{Budget, GuestTicks};

use crate::runtime::state::Runtime;

fn fresh() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(16), Budget::new(4), 100);
    rt.set_hypervisor_firmware("4.93");
    rt
}

/// Allocates `size` bytes in 2^20-byte pages through `rt`'s hypervisor.
fn allocate(rt: &mut Runtime, size: u64) {
    let request = HypercallRequest {
        unit: UnitId::new(0),
        number: ALLOCATE_MEMORY,
        inputs: [size, 20, 0, 0, 0, 0, 0, 0],
        pc: 0,
        lr: 0,
        thread: Some(0),
        tick: GuestTicks::ZERO,
    };
    let disposition = rt.hypervisor.dispatch(&request);
    assert!(
        matches!(disposition, HypercallDisposition::Answered(_)),
        "{disposition:?}"
    );
}

#[test]
fn a_hypervisor_memory_region_moves_the_sync_state_hash() {
    let empty = fresh();
    let mut one = fresh();
    allocate(&mut one, 0x10_0000);
    let mut other = fresh();
    allocate(&mut other, 0x20_0000);
    assert_ne!(one.sync_state_hash(), empty.sync_state_hash());
    assert_ne!(one.sync_state_hash(), other.sync_state_hash());
    for rt in [&empty, &one, &other] {
        assert_eq!(rt.sync_state_hash(), rt.sync_state_hash_from_scratch());
    }
}

#[test]
fn a_restored_snapshot_brings_back_the_hypervisor_memory_hash() {
    let mut rt = fresh();
    let before = rt.snapshot();
    let empty = rt.sync_state_hash();
    allocate(&mut rt, 0x10_0000);
    let allocated = rt.sync_state_hash();
    assert_ne!(allocated, empty);
    rt.restore_into(&before);
    assert_eq!(rt.sync_state_hash(), empty);
    assert!(rt.hypervisor().memory().regions().is_empty());
    allocate(&mut rt, 0x10_0000);
    assert_eq!(rt.sync_state_hash(), allocated);
}
