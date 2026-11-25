//! The logical SPE group's registration, its state and the changes an
//! answer carries to it.

use cellgov_mem::scheme::{HashedSurface, Stamp};

use super::construct::{self, LogicalSpe, LogicalSpeState};
use super::irq_outlet;
use super::transition_notifier::{self, TransitionNotifier, TransitionNotifierState};
use crate::group::state::StateChange;
use crate::group::table::Group;

/// The logical SPE group's registration.
pub(crate) const GROUP: Group = Group {
    name: "spe",
    hypercalls: &[
        construct::HYPERCALLS,
        irq_outlet::HYPERCALLS,
        transition_notifier::HYPERCALLS,
    ],
    lanes: &[192..=207],
    hashed_surfaces: &[
        HashedSurface {
            name: "the logical SPEs",
            source: Some(LOGICAL_SPE_SOURCE),
            stamp: Stamp::Version(27),
        },
        HashedSurface {
            name: "the transition notifier of each logical SPE",
            source: Some(LOGICAL_SPE_NOTIFIER_SOURCE),
            stamp: Stamp::Version(27),
        },
    ],
};

/// The lane source of the logical SPEs the hypervisor constructed, one
/// object per SPE in spe id order.
pub(crate) const LOGICAL_SPE_SOURCE: u8 = 192;

/// The lane source of the transition notifiers of the logical SPEs, one
/// object per notifier in spe id order.
pub(crate) const LOGICAL_SPE_NOTIFIER_SOURCE: u8 = 193;

/// The logical SPE group's state: the partition's logical SPEs and their
/// transition notifiers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpeState {
    spes: LogicalSpeState,
    notifiers: TransitionNotifierState,
}

impl SpeState {
    /// The partition's logical SPEs.
    pub fn spes(&self) -> &LogicalSpeState {
        &self.spes
    }

    /// The transition notifier of each logical SPE.
    pub fn notifiers(&self) -> &TransitionNotifierState {
        &self.notifiers
    }

    /// Applies `change` to the state.
    pub(crate) fn apply(&mut self, change: SpeChange) {
        match change {
            SpeChange::Constructed(spe) => self.spes.construct(spe),
            SpeChange::NotifierSet(notifier) => self.notifiers.set(notifier),
            SpeChange::NotifierCleared(spe) => self.notifiers.clear(spe),
        }
    }

    /// The sync-state terms of the logical SPEs and their notifiers.
    pub fn sync_term(&self) -> u128 {
        self.spes
            .sync_term()
            .wrapping_add(self.notifiers.sync_term())
    }
}

/// A change to the logical SPE group's state that an answer carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeChange {
    /// The call constructed this logical SPE.
    Constructed(LogicalSpe),
    /// The call gave an SPE this transition notifier.
    NotifierSet(TransitionNotifier),
    /// The call removed the transition notifier of the SPE with this spe
    /// id.
    NotifierCleared(u64),
}

impl From<SpeChange> for StateChange {
    fn from(change: SpeChange) -> Self {
        StateChange::Spe(Box::new(change))
    }
}
