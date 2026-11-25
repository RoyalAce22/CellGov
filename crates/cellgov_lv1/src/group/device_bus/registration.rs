//! The device bus group's registration, its state and the changes an
//! answer carries to it.

use std::collections::BTreeSet;

use cellgov_mem::lanes;
use cellgov_mem::scheme::{HashedSurface, Stamp};

use super::open::{self, OpenDevice};
use crate::group::state::StateChange;
use crate::group::table::Group;

/// The device bus group's registration.
pub(crate) const GROUP: Group = Group {
    name: "device_bus",
    hypercalls: &[open::HYPERCALLS],
    lanes: &[176..=191],
    hashed_surfaces: &[HashedSurface {
        name: "the devices that the partition opened",
        source: Some(OPEN_DEVICE_SOURCE),
        stamp: Stamp::Version(31),
    }],
};

/// The lane source of the open devices: one object for each open device,
/// keyed by its index in bus and device order.
pub(crate) const OPEN_DEVICE_SOURCE: u8 = 176;

/// The device bus group's state: each device that the partition opened.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceBusState {
    open: BTreeSet<OpenDevice>,
}

impl DeviceBusState {
    /// Each open device, in bus and device order.
    pub fn open_devices(&self) -> impl Iterator<Item = OpenDevice> + '_ {
        self.open.iter().copied()
    }

    /// Whether the partition opened `device`.
    pub fn is_open(&self, device: OpenDevice) -> bool {
        self.open.contains(&device)
    }

    /// Applies `change` to the state.
    pub(crate) fn apply(&mut self, change: DeviceBusChange) {
        match change {
            DeviceBusChange::Opened(device) => {
                self.open.insert(device);
            }
        }
    }

    /// The sync-state terms of the open devices. A state with no open
    /// device adds no term.
    pub fn sync_term(&self) -> u128 {
        (0u64..)
            .zip(self.open_devices())
            .fold(0u128, |sum, (index, device)| {
                sum.wrapping_add(lanes::value_term(OPEN_DEVICE_SOURCE, index, &device))
            })
    }
}

/// A change to the device bus group's state that an answer carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceBusChange {
    /// The call opened this device.
    Opened(OpenDevice),
}

impl From<DeviceBusChange> for StateChange {
    fn from(change: DeviceBusChange) -> Self {
        StateChange::DeviceBus(Box::new(change))
    }
}

#[cfg(test)]
#[path = "tests/open_device_hash_tests.rs"]
mod hash_tests;
