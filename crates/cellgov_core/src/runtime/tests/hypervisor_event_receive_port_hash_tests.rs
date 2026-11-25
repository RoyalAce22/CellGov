use cellgov_event::UnitId;
use cellgov_lv1::{HypercallDisposition, HypercallRequest};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::call::CONSTRUCT_EVENT_RECEIVE_PORT;
use cellgov_time::{Budget, GuestTicks};

use crate::runtime::state::Runtime;

fn fresh(firmware: &str) -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(16), Budget::new(4), 100);
    rt.set_hypervisor_firmware(firmware);
    rt
}

/// Constructs an event receive port through `rt`'s hypervisor, and
/// returns the disposition.
fn construct(rt: &mut Runtime) -> HypercallDisposition {
    let request = HypercallRequest {
        unit: UnitId::new(0),
        number: CONSTRUCT_EVENT_RECEIVE_PORT,
        inputs: [0; 8],
        pc: 0,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    };
    rt.hypervisor.dispatch(&request)
}

#[test]
fn a_constructed_port_moves_the_sync_state_hash() {
    let empty = fresh("4.93");
    let mut constructed = fresh("4.93");
    assert!(matches!(
        construct(&mut constructed),
        HypercallDisposition::Answered(_)
    ));
    assert_ne!(constructed.sync_state_hash(), empty.sync_state_hash());
    for rt in [&empty, &constructed] {
        assert_eq!(rt.sync_state_hash(), rt.sync_state_hash_from_scratch());
    }
}

#[test]
fn a_refused_port_call_leaves_the_sync_state_hash() {
    let empty = fresh("4.92");
    let mut refused = fresh("4.92");
    assert!(matches!(
        construct(&mut refused),
        HypercallDisposition::Refused(_)
    ));
    assert_eq!(refused.sync_state_hash(), empty.sync_state_hash());
}

#[test]
fn a_restored_snapshot_brings_back_the_port_hash() {
    let mut rt = fresh("4.93");
    let before = rt.snapshot();
    let empty = rt.sync_state_hash();
    construct(&mut rt);
    let constructed = rt.sync_state_hash();
    assert_ne!(constructed, empty);
    rt.restore_into(&before);
    assert_eq!(rt.sync_state_hash(), empty);
    assert!(rt.hypervisor().event_receive_ports().ports().is_empty());
    construct(&mut rt);
    assert_eq!(rt.sync_state_hash(), constructed);
}
