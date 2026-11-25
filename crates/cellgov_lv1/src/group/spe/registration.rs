//! The logical SPE group's registration, its state and the changes an
//! answer carries to it.

use cellgov_mem::scheme::{HashedSurface, Stamp};

use super::construct::{self, LogicalSpe, LogicalSpeState};
use crate::group::state::StateChange;
use crate::group::table::Group;

/// The logical SPE group's registration.
pub(crate) const GROUP: Group = Group {
    name: "spe",
    hypercalls: &[construct::HYPERCALLS],
    lanes: &[192..=207],
    hashed_surfaces: &[HashedSurface {
        name: "the logical SPEs",
        source: Some(LOGICAL_SPE_SOURCE),
        stamp: Stamp::Pending,
    }],
};

/// The lane source of the logical SPEs the hypervisor constructed, one
/// object per SPE in spe id order.
pub(crate) const LOGICAL_SPE_SOURCE: u8 = 192;

/// The logical SPE group's state: the partition's logical SPEs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpeState {
    spes: LogicalSpeState,
}

impl SpeState {
    /// The partition's logical SPEs.
    pub fn spes(&self) -> &LogicalSpeState {
        &self.spes
    }

    /// Applies `change` to the state.
    pub(crate) fn apply(&mut self, change: SpeChange) {
        match change {
            SpeChange::Constructed(spe) => self.spes.construct(spe),
        }
    }

    /// The sync-state terms of the logical SPEs.
    pub fn sync_term(&self) -> u128 {
        self.spes.sync_term()
    }
}

/// A change to the logical SPE group's state that an answer carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeChange {
    /// The call constructed this logical SPE.
    Constructed(LogicalSpe),
}

impl From<SpeChange> for StateChange {
    fn from(change: SpeChange) -> Self {
        StateChange::Spe(Box::new(change))
    }
}
