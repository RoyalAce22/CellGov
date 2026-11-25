//! The virtual UART hypercalls. The group answers no call.

use super::state::StateChange;
use super::table::Group;

/// The virtual UART group's registration.
pub(crate) const GROUP: Group = Group {
    name: "uart",
    hypercalls: &[],
    lanes: &[240..=247],
    hashed_surfaces: &[],
};

/// The virtual UART group's state. It holds nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UartState {}

impl UartState {
    /// Applies `change` to the state.
    pub(crate) fn apply(&mut self, change: UartChange) {
        match change {}
    }

    /// The sync-state terms of the state: none.
    pub fn sync_term(&self) -> u128 {
        0
    }
}

/// A change to the virtual UART group's state. No answer carries one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UartChange {}

impl From<UartChange> for StateChange {
    fn from(change: UartChange) -> Self {
        StateChange::Uart(Box::new(change))
    }
}
