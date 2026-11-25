use cellgov_event::UnitId;
use cellgov_lv2::hypervisor::{HypercallDisposition, HypercallRequest};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::call::{CONSTRUCT_VIRTUAL_ADDRESS_SPACE, SELECT_VIRTUAL_ADDRESS_SPACE};
use cellgov_time::{Budget, GuestTicks};

use crate::runtime::state::Runtime;

/// The vas id that the 4.93 hypervisor gives the kernel's address space.
const VAS_ID: u64 = 2;

fn fresh() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(16), Budget::new(4), 100);
    rt.set_hypervisor_firmware("4.93");
    rt
}

/// Makes hypercall `number` with r3 to r5 `inputs` through `rt`'s
/// hypervisor and checks that it is answered.
fn call(rt: &mut Runtime, number: u64, inputs: [u64; 3]) {
    let mut all = [0; 8];
    all[..3].copy_from_slice(&inputs);
    let request = HypercallRequest {
        unit: UnitId::new(0),
        number,
        inputs: all,
        pc: 0,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    };
    let disposition = rt.hypervisor.dispatch(&request);
    assert!(
        matches!(disposition, HypercallDisposition::Answered(_)),
        "{disposition:?}"
    );
}

/// The 4.93 kernel's construction: r3 = 0, page shifts 16 and 20.
fn construct(rt: &mut Runtime) {
    call(
        rt,
        CONSTRUCT_VIRTUAL_ADDRESS_SPACE,
        [0, 2, 0x1014_0000_0000_0000],
    );
}

#[test]
fn a_constructed_and_a_selected_address_space_each_move_the_sync_state_hash() {
    let empty = fresh();
    let mut constructed = fresh();
    construct(&mut constructed);
    let mut selected = fresh();
    construct(&mut selected);
    call(&mut selected, SELECT_VIRTUAL_ADDRESS_SPACE, [VAS_ID, 0, 0]);
    assert_ne!(constructed.sync_state_hash(), empty.sync_state_hash());
    assert_ne!(selected.sync_state_hash(), constructed.sync_state_hash());
    for rt in [&empty, &constructed, &selected] {
        assert_eq!(rt.sync_state_hash(), rt.sync_state_hash_from_scratch());
    }
}

#[test]
fn a_restored_snapshot_brings_back_the_address_space_hash() {
    let mut rt = fresh();
    let before = rt.snapshot();
    let empty = rt.sync_state_hash();
    construct(&mut rt);
    let constructed = rt.sync_state_hash();
    assert_ne!(constructed, empty);
    rt.restore_into(&before);
    assert_eq!(rt.sync_state_hash(), empty);
    assert!(rt.hypervisor().address_spaces().spaces().is_empty());
    construct(&mut rt);
    assert_eq!(rt.sync_state_hash(), constructed);
}
