//! The network device group's registration, its state and the changes an
//! answer carries to it.

use cellgov_mem::lanes;
use cellgov_mem::scheme::{HashedSurface, Stamp};

use super::indicator;
use crate::group::state::StateChange;
use crate::group::table::Group;

/// The network device group's registration.
pub(crate) const GROUP: Group = Group {
    name: "network",
    hypercalls: &[indicator::HYPERCALLS],
    lanes: &[232..=239],
    hashed_surfaces: &[HashedSurface {
        name: "the network device's interrupt status indicator",
        source: Some(STATUS_INDICATOR_SOURCE),
        stamp: Stamp::Pending,
    }],
};

/// The lane source of the network device's interrupt status indicator:
/// one object, field 1 the indicator's logical partition address.
pub(crate) const STATUS_INDICATOR_SOURCE: u8 = 232;

/// The network device group's state: the interrupt status indicator
/// that the partition set for the network device.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkState {
    indicator: Option<u64>,
}

impl NetworkState {
    /// The logical partition address of the network device's interrupt
    /// status indicator, or `None` when the partition set none.
    pub fn status_indicator(&self) -> Option<u64> {
        self.indicator
    }

    /// Applies `change` to the state.
    pub(crate) fn apply(&mut self, change: NetworkChange) {
        match change {
            NetworkChange::StatusIndicatorSet(address) => self.indicator = Some(address),
        }
    }

    /// The sync-state term of the indicator. A state with no indicator
    /// adds no term.
    pub fn sync_term(&self) -> u128 {
        self.indicator.map_or(0, |address| {
            lanes::value_term(STATUS_INDICATOR_SOURCE, 0, &address)
        })
    }
}

/// A change to the network device group's state that an answer carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkChange {
    /// The call set the interrupt status indicator at this logical
    /// partition address.
    StatusIndicatorSet(u64),
}

impl From<NetworkChange> for StateChange {
    fn from(change: NetworkChange) -> Self {
        StateChange::Network(Box::new(change))
    }
}

#[cfg(test)]
#[path = "tests/status_indicator_hash_tests.rs"]
mod hash_tests;
