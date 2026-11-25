//! The device bus hypercalls: open and close, the MMIO and DMA regions,
//! the I/O segments and the IOMMU entries. The group answers no call.

use super::state::StateChange;
use super::table::Group;

/// The device bus group's registration.
pub(crate) const GROUP: Group = Group {
    name: "device_bus",
    hypercalls: &[],
    lanes: &[176..=191],
    hashed_surfaces: &[],
};

/// The device bus group's state. It holds nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceBusState {}

impl DeviceBusState {
    /// Applies `change` to the state.
    pub(crate) fn apply(&mut self, change: DeviceBusChange) {
        match change {}
    }

    /// The sync-state terms of the state: none.
    pub fn sync_term(&self) -> u128 {
        0
    }
}

/// A change to the device bus group's state. No answer carries one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceBusChange {}

impl From<DeviceBusChange> for StateChange {
    fn from(change: DeviceBusChange) -> Self {
        StateChange::DeviceBus(Box::new(change))
    }
}
