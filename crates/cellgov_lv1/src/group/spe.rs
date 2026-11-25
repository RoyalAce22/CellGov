//! The logical SPE hypercalls, with the isolated SPE calls of the SELF
//! decryption path. No call of the group is answered yet.

use super::state::StateChange;
use super::table::Group;

/// The logical SPE group's registration.
pub(crate) const GROUP: Group = Group {
    name: "spe",
    hypercalls: &[],
    lanes: &[192..=207],
    hashed_surfaces: &[],
};

/// The logical SPE group's state. It holds nothing yet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpeState {}

impl SpeState {
    /// Applies `change` to the state.
    pub(crate) fn apply(&mut self, change: SpeChange) {
        match change {}
    }

    /// The sync-state terms of the state: none yet.
    pub fn sync_term(&self) -> u128 {
        0
    }
}

/// A change to the logical SPE group's state. No answer carries one yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpeChange {}

impl From<SpeChange> for StateChange {
    fn from(change: SpeChange) -> Self {
        StateChange::Spe(Box::new(change))
    }
}
