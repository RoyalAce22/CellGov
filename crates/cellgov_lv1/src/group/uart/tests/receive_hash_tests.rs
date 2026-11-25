//! The virtual UART group hashes the bytes in each receive buffer as one
//! object of its own lane source.

use super::*;
use crate::group::uart::{UartChange, UartState};
use cellgov_mem::lanes::{contribution, LaneIndex};

fn with_received(buffers: &[ReceivedBytes]) -> UartState {
    let mut state = UartState::default();
    for buffer in buffers {
        state.apply(UartChange::ReceiveBufferSet(buffer.clone()));
    }
    state
}

fn received(port: u64, bytes: &[u8]) -> ReceivedBytes {
    ReceivedBytes {
        port,
        bytes: bytes.to_vec(),
    }
}

/// The list holds every nonzero lane of port 10 with 10 bytes.
#[test]
fn received_sync_term_lane_vector_golden() {
    let lane = |field, slot, value| contribution(LaneIndex::new(243, 0, field, slot), value);
    let sum = [
        lane(0, 0, 1),
        lane(1, 0, 10),
        lane(2, 0, 10),
        lane(3, 0, 0x0102_0304_0506_0708),
        lane(3, 1, 0x090A_0000_0000_0000),
    ]
    .into_iter()
    .fold(0u128, u128::wrapping_add);
    let state = with_received(&[received(10, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10])]);
    assert_eq!(state.sync_term(), sum);
    assert_eq!(state.received(10), [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
}

#[test]
fn a_buffer_with_no_byte_removes_the_entry_and_adds_no_term() {
    let state = with_received(&[received(10, &[7; 3]), ReceivedBytes::empty(10)]);
    assert_eq!(state, UartState::default());
    assert_eq!(state.sync_term(), 0);
    assert!(state.received(10).is_empty());
}

/// Both buffers give the same bytes lane, so only the count lane tells
/// them apart.
#[test]
fn a_trailing_zero_byte_changes_the_count_lane() {
    let short = with_received(&[received(10, &[7])]).sync_term();
    let long = with_received(&[received(10, &[7, 0])]).sync_term();
    let lane = |value| contribution(LaneIndex::new(243, 0, 2, 0), value);
    assert_eq!(long.wrapping_sub(short), lane(2).wrapping_sub(lane(1)));
}

#[test]
fn each_buffer_is_one_object_keyed_by_its_index_in_port_order() {
    let state = with_received(&[received(10, &[7]), received(2, &[7])]);
    let object = |key, port| {
        [
            contribution(LaneIndex::new(243, key, 0, 0), 1),
            contribution(LaneIndex::new(243, key, 1, 0), port),
            contribution(LaneIndex::new(243, key, 2, 0), 1),
            contribution(LaneIndex::new(243, key, 3, 0), 0x0700_0000_0000_0000),
        ]
        .into_iter()
        .fold(0u128, u128::wrapping_add)
    };
    assert_eq!(state.sync_term(), object(0, 2).wrapping_add(object(1, 10)));
}
