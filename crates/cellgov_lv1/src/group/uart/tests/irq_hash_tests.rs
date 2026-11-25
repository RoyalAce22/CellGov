//! The virtual UART group hashes its interrupt configuration as one
//! object of its own lane source.

use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

fn configured(outlet: u64, bitmap: u64) -> UartState {
    let mut state = UartState::default();
    state.apply(UartChange::IrqConfigured(VirtualUartIrq { outlet, bitmap }));
    state
}

/// The list holds every nonzero lane of the kernel's configuration:
/// outlet 18 and the bitmap at 0x37dc68.
#[test]
fn configured_irq_sync_term_lane_vector_golden() {
    let lane = |field, value| contribution(LaneIndex::new(240, 0, field, 0), value);
    let sum = [lane(0, 1), lane(1, 18), lane(2, 0x0037_dc68)]
        .into_iter()
        .fold(0u128, u128::wrapping_add);
    assert_eq!(configured(18, 0x0037_dc68).sync_term(), sum);
}

#[test]
fn a_state_with_no_configuration_adds_no_term() {
    assert_eq!(UartState::default().sync_term(), 0);
}

#[test]
fn the_outlet_and_the_bitmap_take_different_lanes() {
    assert_ne!(
        configured(18, 0x40).sync_term(),
        configured(0x40, 18).sync_term()
    );
}
