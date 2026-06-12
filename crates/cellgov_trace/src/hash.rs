//! 64-bit state-hash wrapper used at replay-comparison checkpoints.
//!
//! The committed-memory producer (`GuestMemory::content_hash`) uses FNV-1a;
//! other checkpoint producers pick their own algorithm as long as it is
//! deterministic across hosts.

/// 64-bit deterministic hash of some piece of runtime state.
///
/// Omits `From<u64>` so hash construction stays explicit at call sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct StateHash(u64);

impl StateHash {
    /// Empty-state sentinel.
    pub const ZERO: Self = Self(0);

    /// Construct a `StateHash` from a raw 64-bit value.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Underlying 64-bit value.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
#[path = "tests/hash_tests.rs"]
mod tests;
