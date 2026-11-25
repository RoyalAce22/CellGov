//! The port of the 4.93 kernel's storage bus interrupt, after the port of
//! its network device interrupt.

use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::CONSTRUCT_EVENT_RECEIVE_PORT;
use cellgov_time::GuestTicks;

/// The `sc` of the kernel's call that makes the network device's
/// interrupt port [OFW-4.93 lv2_kernel a:0x80000000000a28e0].
const NETWORK_SC: u64 = 0x8000_0000_000a_28e0;

/// The `sc` of the kernel's port setup of an interrupt object of type 2,
/// which the storage bus function reaches first
/// [OFW-4.93 lv2_kernel a:0x8000000000096494].
const STORAGE_SC: u64 = 0x8000_0000_0009_6494;

/// The id that the console capture `tests/micro/lv1_network_port_id`
/// reads for the network device's port.
const NETWORK_PORT: u64 = 19;

/// The id that the console capture `tests/micro/lv1_storage_port_id`
/// reads for the storage bus's port.
const STORAGE_PORT: u64 = 20;

fn construct(thread: u8, pc: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(0),
        number: CONSTRUCT_EVENT_RECEIVE_PORT,
        inputs: [0xA5A5_A5A5_A5A5_A5A5; 8],
        pc,
        lr: 0,
        thread: Some(thread),
        problem_state: false,
        tick: GuestTicks::new(33_914),
    }
}

fn constructed(id: u64) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [id, 0, 0, 0, 0, 0, 0]);
    answer.push_change(InterruptChange::PortConstructed(EventReceivePort::new(id)));
    HypercallDisposition::Answered(answer)
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: CONSTRUCT_EVENT_RECEIVE_PORT,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

/// A 4.93 hypervisor after the network device's call.
fn after_network_call() -> Hypervisor {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    assert_eq!(
        hv.dispatch(&construct(0, NETWORK_SC)),
        constructed(NETWORK_PORT)
    );
    hv
}

#[test]
fn the_storage_call_gets_the_captured_id_with_r5_to_r10_cleared_and_is_kept() {
    let mut hv = after_network_call();
    let before = hv.sync_term();
    assert_eq!(
        hv.dispatch(&construct(0, STORAGE_SC)),
        constructed(STORAGE_PORT)
    );
    let ids: Vec<u64> = hv.event_receive_ports().ports().keys().copied().collect();
    assert_eq!(ids, [NETWORK_PORT, STORAGE_PORT]);
    assert_eq!(
        hv.event_receive_ports().port(STORAGE_PORT),
        Some(&EventReceivePort::new(STORAGE_PORT))
    );
    assert_ne!(hv.sync_term(), before);
    assert_eq!(hv.refusal(), None);
}

#[test]
fn a_second_call_at_the_storage_sc_is_refused_and_keeps_both_ports() {
    let mut hv = after_network_call();
    assert_eq!(
        hv.dispatch(&construct(0, STORAGE_SC)),
        constructed(STORAGE_PORT)
    );
    let held = hv.event_receive_ports().clone();
    let term = hv.sync_term();
    let r = construct(0, STORAGE_SC);
    assert_eq!(
        hv.dispatch(&r),
        refused(
            &r,
            "an event receive port call whose port id the hypervisor already holds"
        )
    );
    assert_eq!(hv.event_receive_ports(), &held);
    assert_eq!(hv.sync_term(), term);
}

#[test]
fn the_storage_sc_on_thread_1_is_refused() {
    let mut hv = after_network_call();
    let r = construct(1, STORAGE_SC);
    assert_eq!(
        hv.dispatch(&r),
        refused(
            &r,
            "an event receive port call whose port id no source gives for its thread and address"
        )
    );
    let ids: Vec<u64> = hv.event_receive_ports().ports().keys().copied().collect();
    assert_eq!(ids, [NETWORK_PORT]);
}
