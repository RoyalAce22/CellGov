//! The storage device hypercalls: flash, HDD and Blu-ray storage. No
//! call of the group is answered yet.

use super::state::StateChange;
use super::table::Group;

/// The storage group's registration.
pub(crate) const GROUP: Group = Group {
    name: "storage",
    hypercalls: &[],
    lanes: &[208..=215],
    hashed_surfaces: &[],
};

/// The storage group's state. It holds nothing yet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StorageState {}

impl StorageState {
    /// Applies `change` to the state.
    pub(crate) fn apply(&mut self, change: StorageChange) {
        match change {}
    }

    /// The sync-state terms of the state: none yet.
    pub fn sync_term(&self) -> u128 {
        0
    }
}

/// A change to the storage group's state. No answer carries one yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageChange {}

impl From<StorageChange> for StateChange {
    fn from(change: StorageChange) -> Self {
        StateChange::Storage(Box::new(change))
    }
}
