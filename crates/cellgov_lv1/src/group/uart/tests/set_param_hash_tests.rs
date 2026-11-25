//! The virtual UART group hashes the values of each port as one object of
//! its own lane source.

use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

fn with_ports(ports: &[VirtualUartPortParams]) -> UartState {
    let mut state = UartState::default();
    for &params in ports {
        state.apply(UartChange::PortParamsSet(params));
    }
    state
}

fn params(port: u64, tx: u64, rx: u64) -> VirtualUartPortParams {
    VirtualUartPortParams {
        port,
        tx_trigger: tx,
        rx_trigger: rx,
        interrupt_mask: 0,
    }
}

/// The list holds every nonzero lane of the kernel's port 10 after its
/// open: triggers 0xfff and 0xfff.
#[test]
fn port_params_sync_term_lane_vector_golden() {
    let lane = |field, value| contribution(LaneIndex::new(241, 0, field, 0), value);
    let sum = [lane(0, 1), lane(1, 10), lane(2, 0xfff), lane(3, 0xfff)]
        .into_iter()
        .fold(0u128, u128::wrapping_add);
    assert_eq!(with_ports(&[params(10, 0xfff, 0xfff)]).sync_term(), sum);
}

#[test]
fn a_port_reset_to_its_initial_values_adds_no_term() {
    let state = with_ports(&[params(2, 0x7ff, 0x7ff), params(2, 0, 0)]);
    assert_eq!(state, UartState::default());
    assert_eq!(state.sync_term(), 0);
}

#[test]
fn the_two_triggers_take_different_lanes() {
    assert_ne!(
        with_ports(&[params(0, 0x7ff, 0x3ff)]).sync_term(),
        with_ports(&[params(0, 0x3ff, 0x7ff)]).sync_term()
    );
}

#[test]
fn the_interrupt_mask_takes_its_own_lane() {
    let mut masked = params(0, 0x7ff, 0x7ff);
    masked.interrupt_mask = 2;
    let unmasked = with_ports(&[params(0, 0x7ff, 0x7ff)]).sync_term();
    assert_ne!(with_ports(&[masked]).sync_term(), unmasked);
    assert_eq!(
        with_ports(&[masked]).sync_term(),
        unmasked.wrapping_add(contribution(LaneIndex::new(241, 0, 4, 0), 2))
    );
}

/// The second port is the object with key 1 of the lane source.
#[test]
fn each_port_is_one_object_keyed_by_its_index_in_port_order() {
    let state = with_ports(&[params(10, 1, 1), params(0, 2, 2)]);
    let object = |key, port, value| {
        [
            contribution(LaneIndex::new(241, key, 0, 0), 1),
            contribution(LaneIndex::new(241, key, 1, 0), port),
            contribution(LaneIndex::new(241, key, 2, 0), value),
            contribution(LaneIndex::new(241, key, 3, 0), value),
        ]
        .into_iter()
        .fold(0u128, u128::wrapping_add)
    };
    assert_eq!(
        state.sync_term(),
        object(0, 0, 2).wrapping_add(object(1, 10, 1))
    );
}
