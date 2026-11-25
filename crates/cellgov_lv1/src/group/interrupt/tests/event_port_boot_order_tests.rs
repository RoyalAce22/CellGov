//! The port ids of the 4.93 kernel's later calls, which the capture
//! `lv2_event_port_ids` reads, and the connections of those ports.

use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{CONNECT_IRQ_PLUG_EXT, CONSTRUCT_EVENT_RECEIVE_PORT};
use cellgov_ps3_abi::lv1::ppe::game_os_logical_ppe_id;
use cellgov_time::GuestTicks;

/// The `sc` of the call in the setup of a thread's record
/// [OFW-4.93 lv2_kernel a:0x8000000000007d8c].
const RECORD: u64 = 0x8000_0000_0000_7d8c;

/// The `sc` of calls A, B and C of a thread's start
/// [OFW-4.93 lv2_kernel a:0x80000000002656d0]
/// [OFW-4.93 lv2_kernel a:0x8000000000265784]
/// [OFW-4.93 lv2_kernel a:0x8000000000265840].
const START: [u64; 3] = [
    0x8000_0000_0026_56d0,
    0x8000_0000_0026_5784,
    0x8000_0000_0026_5840,
];

/// The `sc` of a later call of hypercall 18 in the kernel, whose port id
/// the capture does not read [OFW-4.93 lv2_kernel a:0x8000000000027d20].
const UNCAPTURED: u64 = 0x8000_0000_0002_7d20;

/// The id of thread 0's record call, and the ids of thread 0's calls A,
/// B and C, as the capture reads them.
const THREAD_0_RECORD_ID: u64 = 3;
const THREAD_0_START_IDS: [u64; 3] = [8, 9, 10];

/// The ids of thread 1's record call and its calls A, B and C, as the
/// capture reads them.
const THREAD_1_IDS: [u64; 4] = [4, 5, 6, 7];

fn kernel_call(number: u64, thread: Option<u8>, pc: u64, args: &[u64]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..args.len()].copy_from_slice(args);
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs,
        pc,
        lr: 0,
        thread,
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

fn construct(thread: Option<u8>, pc: u64) -> HypercallRequest {
    kernel_call(CONSTRUCT_EVENT_RECEIVE_PORT, thread, pc, &[])
}

fn constructed(id: u64) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [id, 0, 0, 0, 0, 0, 0]);
    answer.push_change(InterruptChange::PortConstructed(EventReceivePort::new(id)));
    HypercallDisposition::Answered(answer)
}

fn refused(request: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: CONSTRUCT_EVENT_RECEIVE_PORT,
        unit: request.unit,
        pc: request.pc,
        what,
    })
}

/// A 4.93 hypervisor after thread 0's record call.
fn after_thread_0_record() -> Hypervisor {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    assert_eq!(
        hv.dispatch(&construct(Some(0), RECORD)),
        constructed(THREAD_0_RECORD_ID)
    );
    hv
}

/// A 4.93 hypervisor after thread 0's record call and its start calls.
fn after_thread_0_start() -> Hypervisor {
    let mut hv = after_thread_0_record();
    for (pc, id) in START.into_iter().zip(THREAD_0_START_IDS) {
        assert_eq!(hv.dispatch(&construct(Some(0), pc)), constructed(id));
    }
    hv
}

#[test]
fn thread_0_s_start_calls_get_the_ids_the_capture_reads_after_its_record_call() {
    let mut hv = after_thread_0_record();
    for (pc, id) in START.into_iter().zip(THREAD_0_START_IDS) {
        assert_eq!(
            hv.dispatch(&construct(Some(0), pc)),
            constructed(id),
            "{pc:#018x}"
        );
    }
    let held: Vec<u64> = hv.event_receive_ports().ports().keys().copied().collect();
    assert_eq!(held, [3, 8, 9, 10]);
    assert_eq!(hv.refusal(), None);
}

#[test]
fn thread_1_s_calls_get_the_ids_the_capture_reads() {
    let mut hv = after_thread_0_record();
    for (pc, id) in [RECORD].into_iter().chain(START).zip(THREAD_1_IDS) {
        assert_eq!(
            hv.dispatch(&construct(Some(1), pc)),
            constructed(id),
            "{pc:#018x}"
        );
    }
    assert_eq!(hv.refusal(), None);
}

#[test]
fn each_start_port_of_thread_0_connects_to_a_plug_of_thread_0() {
    let mut hv = after_thread_0_start();
    let ppe_id = game_os_logical_ppe_id("4.93").expect("the 4.93 logical PPE id");
    let thread_0 = IrqStateBitmapKey { ppe_id, thread: 0 };
    for (plug, id) in (0u8..).zip(THREAD_0_START_IDS) {
        let connect = kernel_call(
            CONNECT_IRQ_PLUG_EXT,
            Some(0),
            0,
            &[ppe_id, 0, u64::from(plug), id, 0],
        );
        let HypercallDisposition::Answered(answer) = hv.dispatch(&connect) else {
            panic!("port {id}: the connect is refused");
        };
        assert_eq!(answer.r3, 0, "port {id}: r3");
        assert_eq!(
            hv.event_receive_ports().port(id),
            Some(&EventReceivePort {
                id,
                connection: Some(thread_0),
                plug,
            }),
            "port {id}"
        );
    }
}

#[test]
fn a_call_at_an_address_the_capture_does_not_read_is_refused() {
    let mut hv = after_thread_0_start();
    let r = construct(Some(0), UNCAPTURED);
    assert_eq!(
        hv.dispatch(&r),
        refused(
            &r,
            "an event receive port call whose port id no source gives for its thread and address"
        )
    );
}

#[test]
fn a_call_from_a_unit_that_names_no_hardware_thread_is_refused() {
    let r = construct(None, RECORD);
    assert_eq!(
        construct_port(&r, Some("4.93"), &EventReceivePortState::default()),
        refused(
            &r,
            "an event receive port call whose port id no source gives for its thread and address"
        )
    );
}

#[test]
fn a_repeated_start_call_is_refused_and_leaves_the_connected_port() {
    let mut hv = after_thread_0_record();
    let id = THREAD_0_START_IDS[0];
    assert_eq!(hv.dispatch(&construct(Some(0), START[0])), constructed(id));
    let ppe_id = game_os_logical_ppe_id("4.93").expect("the 4.93 logical PPE id");
    let connect = kernel_call(CONNECT_IRQ_PLUG_EXT, Some(0), 0, &[ppe_id, 0, 0, id, 0]);
    assert!(matches!(
        hv.dispatch(&connect),
        HypercallDisposition::Answered(_)
    ));
    let port = *hv
        .event_receive_ports()
        .port(id)
        .expect("the connected port");
    assert!(port.connection.is_some());
    let again = construct(Some(0), START[0]);
    assert_eq!(
        hv.dispatch(&again),
        refused(
            &again,
            "an event receive port call whose port id the hypervisor already holds"
        )
    );
    assert_eq!(hv.event_receive_ports().port(id), Some(&port));
}
