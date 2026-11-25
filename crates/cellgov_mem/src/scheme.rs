//! The commit-checkpoint hash scheme: the tag that names it, and the
//! hashed surfaces that each version of the tag covers.
//!
//! The scheme covers the committed-memory, unit-status, runnable-queue
//! and sync-state hashes. When a change gives some state a new hash
//! value under one of them, the tag's version moves.
//!
//! A change that adds a hashed surface does not move the tag. It
//! declares the surface with [`Stamp::Pending`]. A later change moves the
//! version once, by one, and stamps each pending surface with the new
//! version. So two changes that each add a surface never edit the tag.

/// The tag that the checkpoint scheme id hashes.
pub const CHECKPOINT_TAG: &[u8] = b"cellgov-checkpoint-fnv1a/v30";

/// The version that [`CHECKPOINT_TAG`] ends with.
pub const CHECKPOINT_VERSION: u32 = tag_version(CHECKPOINT_TAG);

/// The first version that the stamped surfaces account for. Each
/// version from this one to [`CHECKPOINT_VERSION`] is the stamp of at
/// least one surface.
pub const FIRST_STAMPED_VERSION: u32 = 22;

/// The scheme version that first covers a hashed surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stamp {
    /// This version of [`CHECKPOINT_TAG`] covers the surface.
    Version(u32),
    /// No version covers the surface yet. The change that moves the
    /// tag's version stamps the surface with it.
    Pending,
}

/// One hashed surface: a part of the state that a checkpoint hash
/// covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HashedSurface {
    /// What the surface holds.
    pub name: &'static str,
    /// The sync-state lane source the surface hashes under, or `None`
    /// for a surface outside the sync-state lanes.
    pub source: Option<u8>,
    /// The scheme version that first covers the surface.
    pub stamp: Stamp,
}

/// The hashed surfaces outside the hypervisor that a version from
/// [`FIRST_STAMPED_VERSION`] added, or that wait for a version. A
/// hypercall group declares each hypervisor surface itself.
pub const RUNTIME_SURFACES: &[HashedSurface] = &[HashedSurface {
    name: "the inputs queued for each unit's next step",
    source: Some(crate::lanes::source::PENDING_UNIT_INPUT),
    stamp: Stamp::Version(27),
}];

/// The decimal number at the end of `tag`, or 0 when `tag` ends with no
/// digit.
const fn tag_version(tag: &[u8]) -> u32 {
    let mut start = tag.len();
    while start > 0 && tag[start - 1].is_ascii_digit() {
        start -= 1;
    }
    let mut version = 0u32;
    let mut at = start;
    while at < tag.len() {
        version = version * 10 + (tag[at] - b'0') as u32;
        at += 1;
    }
    version
}

#[cfg(test)]
#[path = "tests/scheme_tests.rs"]
mod tests;
