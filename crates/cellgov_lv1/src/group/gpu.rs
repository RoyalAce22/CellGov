//! The GPU hypercalls. The group answers no call.

use super::state::StateChange;
use super::table::Group;

/// The GPU group's registration.
pub(crate) const GROUP: Group = Group {
    name: "gpu",
    hypercalls: &[],
    lanes: &[216..=231],
    hashed_surfaces: &[],
};

/// The GPU group's state. It holds nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GpuState {}

impl GpuState {
    /// Applies `change` to the state.
    pub(crate) fn apply(&mut self, change: GpuChange) {
        match change {}
    }

    /// The sync-state terms of the state: none.
    pub fn sync_term(&self) -> u128 {
        0
    }
}

/// A change to the GPU group's state. No answer carries one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuChange {}

impl From<GpuChange> for StateChange {
    fn from(change: GpuChange) -> Self {
        StateChange::Gpu(Box::new(change))
    }
}
