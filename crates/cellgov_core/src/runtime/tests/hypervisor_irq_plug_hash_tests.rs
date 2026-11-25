use cellgov_event::UnitId;
use cellgov_lv1::{HypercallDisposition, HypercallRequest};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::call::{CONNECT_IRQ_PLUG_EXT, CONSTRUCT_EVENT_RECEIVE_PORT};
use cellgov_time::{Budget, GuestTicks};

use crate::runtime::state::Runtime;

/// The port id of the 4.93 kernel's first port.
const FIRST_PORT_ID: u64 = 3;

/// The `sc` of the kernel's first port call
/// [OFW-4.93 lv2_kernel a:0x8000000000007d8c].
const RECORD_SC: u64 = 0x8000_0000_0000_7d8c;

/// A 4.93 runtime whose hypervisor holds the kernel's first port.
fn with_port() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(16), Budget::new(4), 100);
    rt.set_hypervisor_firmware("4.93");
    let port = HypercallRequest {
        pc: RECORD_SC,
        thread: Some(0),
        ..call(CONSTRUCT_EVENT_RECEIVE_PORT, [0; 5])
    };
    assert!(matches!(
        rt.hypervisor.dispatch(&port),
        HypercallDisposition::Answered(_)
    ));
    rt
}

fn call(number: u64, args: [u64; 5]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..5].copy_from_slice(&args);
    HypercallRequest {
        unit: UnitId::new(0),
        number,
        inputs,
        pc: 0,
        lr: 0,
        thread: Some(1),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

/// Connects the first port to plug 0 of thread 1, which holds no bitmap,
/// and returns the disposition.
fn connect(rt: &mut Runtime) -> HypercallDisposition {
    rt.hypervisor
        .dispatch(&call(CONNECT_IRQ_PLUG_EXT, [1, 1, 0, FIRST_PORT_ID, 0]))
}

#[test]
fn a_connected_port_moves_the_sync_state_hash() {
    let unconnected = with_port();
    let mut connected = with_port();
    assert!(matches!(
        connect(&mut connected),
        HypercallDisposition::Answered(_)
    ));
    assert_ne!(connected.sync_state_hash(), unconnected.sync_state_hash());
    for rt in [&unconnected, &connected] {
        assert_eq!(rt.sync_state_hash(), rt.sync_state_hash_from_scratch());
    }
}

#[test]
fn a_restored_snapshot_brings_back_the_unconnected_port_hash() {
    let mut rt = with_port();
    let before = rt.snapshot();
    let unconnected = rt.sync_state_hash();
    connect(&mut rt);
    let connected = rt.sync_state_hash();
    assert_ne!(connected, unconnected);
    rt.restore_into(&before);
    assert_eq!(rt.sync_state_hash(), unconnected);
    connect(&mut rt);
    assert_eq!(rt.sync_state_hash(), connected);
}
