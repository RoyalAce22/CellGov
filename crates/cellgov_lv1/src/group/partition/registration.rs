//! The partition group's registration, its state and the changes an
//! answer carries to it.

use super::{console, partition_id, ppe, repository};
use crate::group::state::StateChange;
use crate::group::table::Group;

/// The partition group's registration.
pub(crate) const GROUP: Group = Group {
    name: "partition",
    hypercalls: &[
        ppe::HYPERCALLS,
        partition_id::HYPERCALLS,
        repository::HYPERCALLS,
        console::HYPERCALLS,
    ],
    lanes: &[160..=175],
    hashed_surfaces: &[],
};

/// The partition group's state. It holds nothing yet: the repository
/// and the logical console keep no guest state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartitionState {}

impl PartitionState {
    /// Applies `change` to the state.
    pub(crate) fn apply(&mut self, change: PartitionChange) {
        match change {}
    }

    /// The sync-state terms of the state: none yet.
    pub fn sync_term(&self) -> u128 {
        0
    }
}

/// A change to the partition group's state. No answer carries one yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartitionChange {}

impl From<PartitionChange> for StateChange {
    fn from(change: PartitionChange) -> Self {
        StateChange::Partition(Box::new(change))
    }
}
