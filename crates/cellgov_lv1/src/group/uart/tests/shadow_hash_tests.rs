//! The virtual UART group hashes its interrupt shadow as one object of
//! its own lane source.

use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

fn with_shadow(index: u8, value: u64) -> UartState {
    let mut state = UartState::default();
    state.apply(UartChange::ShadowSet(VirtualUartShadowWrite {
        index,
        value,
    }));
    state
}

/// The list holds every nonzero lane of the shadow with port 10's bit,
/// bit 63 - 10 of doubleword 0, which is field 1.
#[test]
fn shadow_sync_term_lane_vector_golden() {
    let lane = |field, value| contribution(LaneIndex::new(244, 0, field, 0), value);
    let sum = lane(0, 1).wrapping_add(lane(1, 1 << 53));
    assert_eq!(with_shadow(0, 1 << 53).sync_term(), sum);
}

#[test]
fn a_shadow_with_no_set_bit_adds_no_term() {
    assert_eq!(with_shadow(0, 0).sync_term(), 0);
}

#[test]
fn each_shadow_doubleword_takes_its_own_lane() {
    assert_ne!(with_shadow(0, 1).sync_term(), with_shadow(3, 1).sync_term());
}
