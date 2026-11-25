//! The virtual UART group hashes the request that is not complete on each
//! port as one object of its own lane source.

use super::*;
use crate::group::uart::{UartChange, UartState};
use cellgov_mem::lanes::{contribution, LaneIndex};

/// The header that the 4.93 kernel sends first in CellGov's boot
/// [OFW-4.93 lv2_kernel a:0x80000000002121f4].
const KERNEL_HEADER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x90, 0x00, 0x00, 0x00, 0x00, 0x38, 0x00, 0x00, 0x00, 0x38,
];

fn with_requests(requests: &[PendingRequest]) -> UartState {
    let mut state = UartState::default();
    for &request in requests {
        state.apply(UartChange::RequestBytesSent(request));
    }
    state
}

/// The list holds every nonzero lane of port 10 after the kernel's header.
#[test]
fn request_sync_term_lane_vector_golden() {
    let lane = |field, value| contribution(LaneIndex::new(242, 0, field, 0), value);
    let sum = [
        lane(0, 1),
        lane(1, 10),
        lane(2, 16),
        lane(3, 0x0000_0000_0001_9000),
        lane(4, 0x0000_0038_0000_0038),
    ]
    .into_iter()
    .fold(0u128, u128::wrapping_add);
    let request = PendingRequest::empty(10).with(&KERNEL_HEADER);
    assert_eq!(with_requests(&[request]).sync_term(), sum);
}

#[test]
fn a_request_with_no_byte_removes_the_entry_and_adds_no_term() {
    let state = with_requests(&[
        PendingRequest::empty(10).with(&KERNEL_HEADER),
        PendingRequest::empty(10),
    ]);
    assert_eq!(state, UartState::default());
    assert_eq!(state.sync_term(), 0);
}

#[test]
fn the_count_sent_and_each_prefix_doubleword_take_their_own_lanes() {
    let base = PendingRequest::empty(10).with(&KERNEL_HEADER);
    let more = base.with(&[0; 8]);
    let body = base.with(&[0, 0, 0, 0, 0, 1, 0x90, 2]);
    let terms = [base, more, body].map(|r| with_requests(&[r]).sync_term());
    let lane = |field, value| contribution(LaneIndex::new(242, 0, field, 0), value);
    assert_eq!(
        terms[1].wrapping_sub(terms[0]),
        lane(2, 24).wrapping_sub(lane(2, 16))
    );
    assert_eq!(
        terms[2].wrapping_sub(terms[1]),
        lane(5, 0x0000_0000_0001_9002)
    );
}

/// The second port is the object with key 1 of the lane source.
#[test]
fn each_request_is_one_object_keyed_by_its_index_in_port_order() {
    let state = with_requests(&[
        PendingRequest::empty(10).with(&[7]),
        PendingRequest::empty(2).with(&[7]),
    ]);
    let object = |key, port| {
        [
            contribution(LaneIndex::new(242, key, 0, 0), 1),
            contribution(LaneIndex::new(242, key, 1, 0), port),
            contribution(LaneIndex::new(242, key, 2, 0), 1),
            contribution(LaneIndex::new(242, key, 3, 0), 0x0700_0000_0000_0000),
        ]
        .into_iter()
        .fold(0u128, u128::wrapping_add)
    };
    assert_eq!(state.sync_term(), object(0, 2).wrapping_add(object(1, 10)));
}
