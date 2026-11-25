//! The network device group's registration, its state and the changes an
//! answer carries to it.

use cellgov_mem::lanes;
use cellgov_mem::scheme::{HashedSurface, Stamp};

use super::{control, indicator, interrupt_mask, interrupt_port};
use crate::group::state::StateChange;
use crate::group::table::Group;

/// The network device group's registration.
pub(crate) const GROUP: Group = Group {
    name: "network",
    hypercalls: &[
        indicator::HYPERCALLS,
        interrupt_port::HYPERCALLS,
        control::HYPERCALLS,
        interrupt_mask::HYPERCALLS,
    ],
    lanes: &[232..=239],
    hashed_surfaces: &[
        HashedSurface {
            name: "the network device's interrupt status indicator",
            source: Some(STATUS_INDICATOR_SOURCE),
            stamp: Stamp::Version(32),
        },
        HashedSurface {
            name: "the event receive port of the network device's interrupt",
            source: Some(INTERRUPT_PORT_SOURCE),
            stamp: Stamp::Version(32),
        },
        HashedSurface {
            name: "the network device's interrupt mask",
            source: Some(INTERRUPT_MASK_SOURCE),
            stamp: Stamp::Version(32),
        },
    ],
};

/// The lane source of the network device's interrupt status indicator:
/// one object, whose field 1 holds the indicator's logical partition
/// address.
pub(crate) const STATUS_INDICATOR_SOURCE: u8 = 232;

/// The lane source of the event receive port that lv1 connected to the
/// network device's interrupt: one object, whose field 1 holds the port
/// id.
pub(crate) const INTERRUPT_PORT_SOURCE: u8 = 233;

/// The lane source of the network device's interrupt mask: one object,
/// whose field 1 holds the mask.
pub(crate) const INTERRUPT_MASK_SOURCE: u8 = 234;

/// The network device group's state.
///
/// The state holds these values:
/// - the interrupt status indicator that the partition set;
/// - the event receive port that lv1 connected to the device's interrupt;
/// - the interrupt mask that the partition set last.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkState {
    indicator: Option<u64>,
    interrupt_port: Option<u64>,
    interrupt_mask: Option<u64>,
}

impl NetworkState {
    /// The logical partition address of the network device's interrupt
    /// status indicator, or `None` when the partition set none.
    pub fn status_indicator(&self) -> Option<u64> {
        self.indicator
    }

    /// The id of the event receive port that lv1 connected to the network
    /// device's interrupt, or `None` when lv1 connected no port.
    pub fn interrupt_port(&self) -> Option<u64> {
        self.interrupt_port
    }

    /// The network device's interrupt mask that the partition set last, or
    /// `None` when the partition set none.
    pub fn interrupt_mask(&self) -> Option<u64> {
        self.interrupt_mask
    }

    /// Applies `change` to the state.
    pub(crate) fn apply(&mut self, change: NetworkChange) {
        match change {
            NetworkChange::StatusIndicatorSet(address) => self.indicator = Some(address),
            NetworkChange::InterruptPortConnected(id) => self.interrupt_port = Some(id),
            NetworkChange::InterruptMaskSet(mask) => self.interrupt_mask = Some(mask),
        }
    }

    /// The sync-state terms of the indicator, the interrupt port and the
    /// interrupt mask. A value that is not set adds no term.
    pub fn sync_term(&self) -> u128 {
        let indicator = self.indicator.map_or(0, |address| {
            lanes::value_term(STATUS_INDICATOR_SOURCE, 0, &address)
        });
        let port = self
            .interrupt_port
            .map_or(0, |id| lanes::value_term(INTERRUPT_PORT_SOURCE, 0, &id));
        let mask = self
            .interrupt_mask
            .map_or(0, |mask| lanes::value_term(INTERRUPT_MASK_SOURCE, 0, &mask));
        indicator.wrapping_add(port).wrapping_add(mask)
    }
}

/// A change to the network device group's state that an answer carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkChange {
    /// The call set the interrupt status indicator at this logical
    /// partition address.
    StatusIndicatorSet(u64),
    /// The call connected the event receive port with this id to the
    /// network device's interrupt.
    InterruptPortConnected(u64),
    /// The call set this interrupt mask for the network device.
    InterruptMaskSet(u64),
}

impl From<NetworkChange> for StateChange {
    fn from(change: NetworkChange) -> Self {
        StateChange::Network(Box::new(change))
    }
}

#[cfg(test)]
#[path = "tests/status_indicator_hash_tests.rs"]
mod hash_tests;

#[cfg(test)]
#[path = "tests/interrupt_port_hash_tests.rs"]
mod interrupt_port_hash_tests;

#[cfg(test)]
#[path = "tests/interrupt_mask_hash_tests.rs"]
mod interrupt_mask_hash_tests;
