//! The network device's state word, which lv1's network device
//! hypercalls check after the bus and device checks and before their
//! other inputs.
//!
//! lv1 keeps the word at +8 of the object at +16 of the network device's
//! global object; see [`cellgov_ps3_abi::lv1::network`]. The answers of
//! `net_set_interrupt_status_indicator`, `net_control` and
//! `net_set_interrupt_mask` read it through [`state_word`].

use super::indicator::NETWORK_DEVICE;
use super::NetworkState;
use crate::group::device_bus::DeviceBusState;

/// The network device's state word as the model knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StateWord {
    /// The word is 2: the partition holds the device open, and the open's
    /// setup stored 2 [OFW-4.93 lv1 a:0x23E1C8-0x23E1D4].
    Set,
    /// The word is 3: the close's teardown stored it
    /// [OFW-4.93 lv1 a:0x23E0F8-0x23E108], and no open came after it.
    TornDown,
    /// The partition never opened the device. lv1's init sets the word,
    /// and no source gives it.
    Unread,
}

/// The network device's state word for the open set of `devices` and the
/// teardown that `network` holds.
pub(super) fn state_word(devices: &DeviceBusState, network: &NetworkState) -> StateWord {
    if devices.is_open(NETWORK_DEVICE) {
        StateWord::Set
    } else if network.torn_down() {
        StateWord::TornDown
    } else {
        StateWord::Unread
    }
}

#[cfg(test)]
#[path = "tests/state_word_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/after_close_tests.rs"]
mod after_close_tests;

#[cfg(test)]
#[path = "tests/console_state_word_tests.rs"]
mod console_state_word_tests;
