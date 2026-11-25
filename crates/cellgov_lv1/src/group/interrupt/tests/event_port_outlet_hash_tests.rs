//! The state keeps the irq outlets apart from the ports and hashes them
//! under a lane source of their own. A port call does not take the id of
//! an outlet.

use super::*;
use cellgov_event::UnitId;
use cellgov_mem::lanes::{contribution, LaneIndex};
use cellgov_time::GuestTicks;

fn outlets(outlets: &[EventReceivePort]) -> EventReceivePortState {
    let mut state = EventReceivePortState::default();
    for &outlet in outlets {
        state.construct_outlet(outlet);
    }
    state
}

/// The list holds every nonzero lane of a state with one outlet, of id
/// 13, connected to plug 9 of thread 7 of the logical PPE 5. Each field
/// has a different value, so a field that moves to another lane changes
/// the sum.
#[test]
fn connected_outlet_sync_term_lane_vector_golden() {
    let lane = |field, value| contribution(LaneIndex::new(144, 0, field, 0), value);
    let state = outlets(&[EventReceivePort {
        id: 13,
        connection: Some(IrqStateBitmapKey {
            ppe_id: 5,
            thread: 7,
        }),
        plug: 9,
    }]);
    let sum = [
        lane(0, 1),
        lane(1, 13),
        lane(2, 1),
        lane(3, 5),
        lane(4, 7),
        lane(5, 9),
    ]
    .into_iter()
    .fold(0u128, u128::wrapping_add);
    assert_eq!(state.sync_term(), sum);
}

/// The list holds every nonzero lane of a state with the outlets 15 and
/// 13, constructed in that order: id 13 is object 0 and id 15 is object 1.
#[test]
fn two_outlets_take_objects_0_and_1_in_id_order_golden() {
    let lane = |object, field, value| contribution(LaneIndex::new(144, object, field, 0), value);
    let sum = [lane(0, 0, 1), lane(0, 1, 13), lane(1, 0, 1), lane(1, 1, 15)]
        .into_iter()
        .fold(0u128, u128::wrapping_add);
    assert_eq!(
        outlets(&[EventReceivePort::new(15), EventReceivePort::new(13)]).sync_term(),
        sum
    );
}

#[test]
fn an_outlet_is_not_a_port_and_a_port_is_not_an_outlet() {
    let mut state = outlets(&[EventReceivePort::new(13)]);
    state.construct(EventReceivePort::new(16));
    assert_eq!(state.port(13), None);
    assert_eq!(state.outlet(13), Some(&EventReceivePort::new(13)));
    assert_eq!(state.outlet(16), None);
    assert_eq!(state.port(16), Some(&EventReceivePort::new(16)));
}

#[test]
fn an_outlet_and_a_port_with_the_same_fields_give_different_terms() {
    let mut port = EventReceivePortState::default();
    port.construct(EventReceivePort::new(13));
    assert_ne!(
        outlets(&[EventReceivePort::new(13)]).sync_term(),
        port.sync_term()
    );
}

#[test]
fn a_connect_step_replaces_the_outlet_moves_its_term_and_adds_no_port() {
    let mut state = outlets(&[EventReceivePort::new(13)]);
    let unconnected = state.sync_term();
    let connected = EventReceivePort {
        id: 13,
        connection: Some(IrqStateBitmapKey {
            ppe_id: 1,
            thread: 0,
        }),
        plug: 4,
    };
    state.connect_outlet(connected);
    assert_eq!(state.outlet(13), Some(&connected));
    assert_eq!(state.outlets().len(), 1);
    assert_ne!(state.sync_term(), unconnected);
    assert_eq!(state.ports(), &BTreeMap::new());
}

/// The 4.93 kernel's port call in the setup of the record of thread 0
/// [OFW-4.93 lv2_kernel a:0x8000000000007d8c], with an outlet that holds
/// the id that the capture gives that call.
#[test]
fn a_port_call_whose_id_an_outlet_holds_is_refused() {
    let r = HypercallRequest {
        unit: UnitId::new(1),
        number: CONSTRUCT_EVENT_RECEIVE_PORT,
        inputs: [0; 8],
        pc: 0x8000_0000_0000_7d8c,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    };
    let id = game_os_port_id("4.93", 0, r.pc).expect("the port id of thread 0's record call");
    assert_eq!(
        construct_port(&r, Some("4.93"), &outlets(&[EventReceivePort::new(id)])),
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
            number: CONSTRUCT_EVENT_RECEIVE_PORT,
            unit: r.unit,
            pc: r.pc,
            what: "an event receive port call whose port id the hypervisor already holds",
        })
    );
}
