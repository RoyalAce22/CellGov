//! The GPU group's registration, its state and the changes an answer
//! carries to it.

use cellgov_mem::lanes;
use cellgov_mem::scheme::{HashedSurface, Stamp};

use super::open::{self, GpuOpen};
use crate::group::state::StateChange;
use crate::group::table::Group;

/// The GPU group's registration.
pub(crate) const GROUP: Group = Group {
    name: "gpu",
    hypercalls: &[open::HYPERCALLS],
    lanes: &[216..=231],
    hashed_surfaces: &[HashedSurface {
        name: "the GPU open that the partition holds",
        source: Some(GPU_OPEN_SOURCE),
        stamp: Stamp::Pending,
    }],
};

/// The lane source of the GPU open: object 0 while the partition holds
/// the GPU open.
pub(crate) const GPU_OPEN_SOURCE: u8 = 216;

/// The GPU group's state: the GPU open that the partition holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GpuState {
    open: Option<GpuOpen>,
}

impl GpuState {
    /// The GPU open that the partition holds, if any.
    pub fn gpu_open(&self) -> Option<GpuOpen> {
        self.open
    }

    /// Applies `change` to the state.
    pub(crate) fn apply(&mut self, change: GpuChange) {
        match change {
            GpuChange::Opened(open) => self.open = Some(open),
        }
    }

    /// The sync-state term of the GPU open. A state with no open adds no
    /// term.
    pub fn sync_term(&self) -> u128 {
        self.open
            .map_or(0, |open| lanes::value_term(GPU_OPEN_SOURCE, 0, &open))
    }
}

/// A change to the GPU group's state that an answer carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuChange {
    /// The call opened the GPU for the partition.
    Opened(GpuOpen),
}

impl From<GpuChange> for StateChange {
    fn from(change: GpuChange) -> Self {
        StateChange::Gpu(Box::new(change))
    }
}

#[cfg(test)]
#[path = "tests/open_hash_tests.rs"]
mod hash_tests;
