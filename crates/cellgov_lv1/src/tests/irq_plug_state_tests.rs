use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_mem::lanes::{contribution, source, LaneIndex};
use cellgov_ps3_abi::lv1::call::{
    CONFIGURE_IRQ_STATE_BITMAP, CONNECT_IRQ_PLUG_EXT, CONSTRUCT_EVENT_RECEIVE_PORT,
};
use cellgov_time::GuestTicks;

/// The port id of the 4.93 kernel's first port.
const FIRST_PORT_ID: u64 = 3;

/// The bitmap of the 4.93 kernel's thread 0: +0x1C0 of its record, which
/// the capture `lv2_logical_ppe_record` places at 0x8000000000368f80.
const THREAD_0_BITMAP: u64 = 0x0036_9140;

fn storage() -> GuestMemory {
    GuestMemory::new(0x0037_0000)
}

fn call(number: u64, args: &[u64]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..args.len()].copy_from_slice(args);
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs,
        pc: 0,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

/// A 4.93 hypervisor after the kernel's thread 0 calls 11 and 18.
fn booted() -> Hypervisor {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let bitmap = call(CONFIGURE_IRQ_STATE_BITMAP, &[1, 0, THREAD_0_BITMAP]);
    assert!(matches!(
        hv.dispatch_with(&bitmap, &storage()),
        HypercallDisposition::Answered(_)
    ));
    assert!(matches!(
        hv.dispatch(&call(CONSTRUCT_EVENT_RECEIVE_PORT, &[])),
        HypercallDisposition::Answered(_)
    ));
    hv
}

fn connect(hv: &mut Hypervisor, args: &[u64]) -> HypercallDisposition {
    hv.dispatch_with(&call(CONNECT_IRQ_PLUG_EXT, args), &storage())
}

fn thread_0() -> IrqStateBitmapKey {
    IrqStateBitmapKey {
        ppe_id: 1,
        thread: 0,
    }
}

#[test]
fn the_hypervisor_keeps_the_connection_in_the_port_and_hashes_it() {
    let mut hv = booted();
    let before = hv.sync_term();
    assert!(matches!(
        connect(&mut hv, &[1, 0, 9, FIRST_PORT_ID, 0]),
        HypercallDisposition::Answered(HypercallAnswer { r3: 0, .. })
    ));
    let port = EventReceivePort {
        id: FIRST_PORT_ID,
        connection: Some(thread_0()),
        plug: 9,
    };
    assert_eq!(hv.event_receive_ports().port(FIRST_PORT_ID), Some(&port));
    assert_eq!(
        hv.event_receive_ports().port_on_plug(thread_0(), 9),
        Some(&port)
    );
    assert_ne!(hv.sync_term(), before);
    assert!(hv.witnesses().is_empty());
    assert_eq!(hv.refusal(), None);
}

#[test]
fn a_second_connect_of_the_port_changes_nothing() {
    let mut hv = booted();
    connect(&mut hv, &[1, 0, 9, FIRST_PORT_ID, 0]);
    let once = (hv.event_receive_ports().clone(), hv.sync_term());
    assert!(matches!(
        connect(&mut hv, &[1, 0, 10, FIRST_PORT_ID, 0]),
        HypercallDisposition::Answered(HypercallAnswer {
            r3: IRQ_PLUG_PORT_CONNECTED_R3,
            ..
        })
    ));
    assert_eq!((hv.event_receive_ports().clone(), hv.sync_term()), once);
}

#[test]
fn a_failed_check_and_a_refusal_keep_the_port_unconnected() {
    for args in [
        [1, 2, 0, FIRST_PORT_ID, 0],
        [1, 0, 0, FIRST_PORT_ID, 1],
        [1, 0, 0, 4, 0],
    ] {
        let mut hv = booted();
        let before = hv.sync_term();
        connect(&mut hv, &args);
        assert_eq!(
            hv.event_receive_ports().port(FIRST_PORT_ID),
            Some(&EventReceivePort::new(FIRST_PORT_ID)),
            "{args:?}"
        );
        assert_eq!(hv.sync_term(), before, "{args:?}");
    }
}

#[test]
fn the_plug_of_an_unconnected_port_moves_the_hash() {
    let mut ports = EventReceivePortState::default();
    ports.construct(EventReceivePort::new(FIRST_PORT_ID));
    let unconnected = ports.sync_term();
    ports.connect(EventReceivePort {
        plug: 5,
        ..EventReceivePort::new(FIRST_PORT_ID)
    });
    assert_ne!(ports.sync_term(), unconnected);
    assert_eq!(ports.port_on_plug(thread_0(), 5), None);
}

/// The list holds every nonzero lane of a state with one port, of id 3,
/// connected to plug 9 of thread 1 of the logical PPE 1.
#[test]
fn connected_port_sync_term_lane_vector_golden() {
    let lane = |field, value| {
        contribution(
            LaneIndex::new(source::HYPERVISOR_EVENT_RECEIVE_PORT, 0, field, 0),
            value,
        )
    };
    let mut ports = EventReceivePortState::default();
    ports.construct(EventReceivePort {
        id: 3,
        connection: Some(IrqStateBitmapKey {
            ppe_id: 1,
            thread: 1,
        }),
        plug: 9,
    });
    let sum = [
        lane(0, 1),
        lane(1, 3),
        lane(2, 1),
        lane(3, 1),
        lane(4, 1),
        lane(5, 9),
    ]
    .into_iter()
    .fold(0u128, u128::wrapping_add);
    assert_eq!(ports.sync_term(), sum);
}

#[test]
fn each_connection_field_moves_the_term() {
    let term = |connection, plug| {
        let mut ports = EventReceivePortState::default();
        ports.construct(EventReceivePort {
            id: 3,
            connection,
            plug,
        });
        ports.sync_term()
    };
    let key = |ppe_id, thread| Some(IrqStateBitmapKey { ppe_id, thread });
    let base = term(key(1, 0), 0);
    assert_ne!(base, term(None, 0));
    assert_ne!(base, term(key(1, 1), 0));
    assert_ne!(base, term(key(2, 0), 0));
    assert_ne!(base, term(key(1, 0), 1));
}
