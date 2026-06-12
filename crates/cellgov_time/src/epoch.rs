//! Commit-batch counter used as the granularity for state hashes.

/// A commit-batch counter; advances once per closed batch.
///
/// Within one epoch the commit pipeline's apply pass is atomic:
/// all writes commit together or none do. Post-apply event
/// injection and the epoch advance close the batch. State hashes
/// are taken at epoch boundaries.
///
/// ```compile_fail
/// use cellgov_time::{Epoch, GuestTicks};
/// let _: GuestTicks = Epoch::ZERO.into();
/// ```
///
/// ```compile_fail
/// use cellgov_time::{Epoch, Budget};
/// let _: Budget = Epoch::ZERO.into();
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, derive_more::Display)]
#[display("{_0}")]
pub struct Epoch(u64);

impl Epoch {
    /// Origin; before any commit has run.
    pub const ZERO: Self = Self(0);

    /// Lift a raw count into an `Epoch`.
    #[doc(hidden)]
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Underlying epoch number.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Successor epoch, `None` on overflow.
    #[must_use = "next() returns the successor; assign or pattern-match it"]
    #[inline]
    pub const fn next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }

    /// Advance in place to the successor epoch.
    ///
    /// # Panics
    ///
    /// Panics on `u64` overflow.
    #[inline]
    pub fn advance(&mut self) {
        *self = self.next().expect(
            "epoch overflow at u64::MAX: advancing would fold distinct committed batches \
             into the same sync_state_hash",
        );
    }
}

#[cfg(test)]
#[path = "tests/epoch_tests.rs"]
mod tests;
