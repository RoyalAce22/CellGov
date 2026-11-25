use super::*;
use crate::group::device_bus::{DeviceBusChange, OpenDevice};
use crate::group::network::NetworkChange;

const DEVICE_1: OpenDevice = OpenDevice { bus: 1, device: 1 };

fn devices(open: &[OpenDevice]) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    for device in open {
        state.apply(DeviceBusChange::Opened(*device));
    }
    state
}

fn network(changes: &[NetworkChange]) -> NetworkState {
    let mut state = NetworkState::default();
    for change in changes {
        state.apply(change.clone());
    }
    state
}

#[test]
fn a_device_that_was_never_opened_has_a_state_word_that_no_source_gives() {
    assert_eq!(
        state_word(&DeviceBusState::default(), &NetworkState::default()),
        StateWord::Unread
    );
    assert_eq!(
        state_word(&devices(&[DEVICE_1]), &NetworkState::default()),
        StateWord::Unread
    );
}

/// [OFW-4.93 lv1 a:0x23E1C8-0x23E1D4] stores 2 at the open.
#[test]
fn an_open_device_has_the_state_word_2_after_a_teardown_too() {
    let open = devices(&[NETWORK_DEVICE]);
    for changes in [
        &[][..],
        &[NetworkChange::TornDown][..],
        &[NetworkChange::TornDown, NetworkChange::SetUp][..],
    ] {
        assert_eq!(
            state_word(&open, &network(changes)),
            StateWord::Set,
            "{changes:?}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x23E0F8-0x23E108] stores 3 at the close.
#[test]
fn a_closed_device_after_a_teardown_has_the_state_word_3() {
    let torn_down = network(&[NetworkChange::SetUp, NetworkChange::TornDown]);
    assert_eq!(
        state_word(&DeviceBusState::default(), &torn_down),
        StateWord::TornDown
    );
    assert_eq!(
        state_word(&devices(&[DEVICE_1]), &torn_down),
        StateWord::TornDown
    );
}
