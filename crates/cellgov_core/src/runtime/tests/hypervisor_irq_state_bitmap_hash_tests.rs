use cellgov_event::UnitId;
use cellgov_lv1::{HypercallDisposition, HypercallRequest};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::call::CONFIGURE_IRQ_STATE_BITMAP;
use cellgov_time::{Budget, GuestTicks};

use crate::runtime::state::Runtime;

/// The bitmap of the 4.93 kernel's thread 0: +0x1C0 of its record at
/// 0x8000000000368f80.
const THREAD_0_BITMAP: u64 = 0x0036_9140;

fn fresh() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(16), Budget::new(4), 100);
    rt.set_hypervisor_firmware("4.93");
    rt
}

/// Registers the bitmap at `bitmap` for thread 0 of logical PPE 1
/// through `rt`'s hypervisor, and checks that the call is answered.
fn register(rt: &mut Runtime, bitmap: u64) {
    let mut inputs = [0; 8];
    inputs[..3].copy_from_slice(&[1, 0, bitmap]);
    let request = HypercallRequest {
        unit: UnitId::new(0),
        number: CONFIGURE_IRQ_STATE_BITMAP,
        inputs,
        pc: 0,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    };
    let storage = GuestMemory::new(0x0037_0000);
    let disposition = rt.hypervisor.dispatch_with(&request, &storage);
    assert!(
        matches!(disposition, HypercallDisposition::Answered(_)),
        "{disposition:?}"
    );
}

#[test]
fn a_registered_bitmap_and_its_address_each_move_the_sync_state_hash() {
    let empty = fresh();
    let mut registered = fresh();
    register(&mut registered, THREAD_0_BITMAP);
    let mut moved = fresh();
    register(&mut moved, THREAD_0_BITMAP + 0x40);
    assert_ne!(registered.sync_state_hash(), empty.sync_state_hash());
    assert_ne!(moved.sync_state_hash(), registered.sync_state_hash());
    for rt in [&empty, &registered, &moved] {
        assert_eq!(rt.sync_state_hash(), rt.sync_state_hash_from_scratch());
    }
}

#[test]
fn a_restored_snapshot_brings_back_the_bitmap_hash() {
    let mut rt = fresh();
    let before = rt.snapshot();
    let empty = rt.sync_state_hash();
    register(&mut rt, THREAD_0_BITMAP);
    let registered = rt.sync_state_hash();
    assert_ne!(registered, empty);
    rt.restore_into(&before);
    assert_eq!(rt.sync_state_hash(), empty);
    assert!(rt.hypervisor().irq_state_bitmaps().bitmaps().is_empty());
    register(&mut rt, THREAD_0_BITMAP);
    assert_eq!(rt.sync_state_hash(), registered);
}
