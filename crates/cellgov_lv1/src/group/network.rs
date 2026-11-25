//! The network device hypercalls. No call of the group is answered
//! yet.

use super::state::StateChange;
use super::table::Group;

/// The network device group's registration.
pub(crate) const GROUP: Group = Group {
    name: "network",
    hypercalls: &[],
    lanes: &[232..=239],
    hashed_surfaces: &[],
};

/// The network device group's state. It holds nothing yet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkState {}

impl NetworkState {
    /// Applies `change` to the state.
    pub(crate) fn apply(&mut self, change: NetworkChange) {
        match change {}
    }

    /// The sync-state terms of the state: none yet.
    pub fn sync_term(&self) -> u128 {
        0
    }
}

/// A change to the network device group's state. No answer carries one yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkChange {}

impl From<NetworkChange> for StateChange {
    fn from(change: NetworkChange) -> Self {
        StateChange::Network(Box::new(change))
    }
}
