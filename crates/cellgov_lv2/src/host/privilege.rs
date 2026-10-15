//! A process's privilege, read from its SELF capability word.

use cellgov_ps3_abi::format::sce::{
    CTRL_FLAGS1_DEBUG_MASK, CTRL_FLAGS1_DEBUG_OR_ROOT_MASK, CTRL_FLAGS1_ROOT_MASK,
};

/// The privilege a process's `control_flags1` grants.
///
/// The three predicates share bits: root implies debug-or-root, and
/// the debug mask overlaps both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Privilege(u32);

impl Privilege {
    /// The privilege `control_flags1` grants.
    pub(crate) fn from_control_flags1(control_flags1: u32) -> Self {
        Self(control_flags1)
    }

    /// Whether the process holds root privilege.
    pub fn root(self) -> bool {
        self.0 & CTRL_FLAGS1_ROOT_MASK != 0
    }

    /// Whether the process holds debug or root privilege; the widest of
    /// the three masks.
    pub fn debug_or_root(self) -> bool {
        self.0 & CTRL_FLAGS1_DEBUG_OR_ROOT_MASK != 0
    }

    /// Whether the process holds debug privilege.
    pub fn debug(self) -> bool {
        self.0 & CTRL_FLAGS1_DEBUG_MASK != 0
    }
}
