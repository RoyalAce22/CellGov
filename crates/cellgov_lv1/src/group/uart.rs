//! The virtual UART hypercalls. No call of the group is answered yet.

use super::state::StateChange;
use super::table::Group;

/// The virtual UART group's registration.
pub(crate) const GROUP: Group = Group {
    name: "uart",
    hypercalls: &[],
    lanes: &[240..=247],
    hashed_surfaces: &[],
};

/// The virtual UART group's state. It holds nothing yet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UartState {}

impl UartState {
    /// Applies `change` to the state.
    pub(crate) fn apply(&mut self, change: UartChange) {
        match change {}
    }

    /// The sync-state terms of the state: none yet.
    pub fn sync_term(&self) -> u128 {
        0
    }
}

/// A change to the virtual UART group's state. No answer carries one yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UartChange {}

impl From<UartChange> for StateChange {
    fn from(change: UartChange) -> Self {
        StateChange::Uart(Box::new(change))
    }
}
