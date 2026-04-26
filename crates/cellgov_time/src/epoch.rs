//! Commit-batch counter used as the granularity for state hashes.

use core::fmt;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

impl fmt::Display for Epoch {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_is_origin() {
        assert_eq!(Epoch::ZERO.raw(), 0);
        assert_eq!(Epoch::ZERO, Epoch::new(0));
    }

    #[test]
    fn next_advances_by_one() {
        assert_eq!(Epoch::ZERO.next(), Some(Epoch::new(1)));
        assert_eq!(Epoch::new(41).next(), Some(Epoch::new(42)));
    }

    #[test]
    fn next_at_max_minus_one_yields_max() {
        assert_eq!(Epoch::new(u64::MAX - 1).next(), Some(Epoch::new(u64::MAX)),);
    }

    #[test]
    fn next_at_max_is_none() {
        assert_eq!(Epoch::new(u64::MAX).next(), None);
    }

    #[test]
    fn advance_steps_in_place() {
        let mut e = Epoch::ZERO;
        e.advance();
        assert_eq!(e, Epoch::new(1));
        e.advance();
        assert_eq!(e, Epoch::new(2));
    }

    #[test]
    #[should_panic(expected = "epoch overflow")]
    fn advance_at_max_panics() {
        let mut e = Epoch::new(u64::MAX);
        e.advance();
    }

    #[test]
    fn ordering_is_total_and_monotonic() {
        assert!(Epoch::new(0) < Epoch::new(1));
        assert!(Epoch::new(100) > Epoch::new(99));
    }

    #[test]
    fn ordering_holds_at_max_boundary() {
        assert!(Epoch::new(u64::MAX - 1) < Epoch::new(u64::MAX));
    }

    #[test]
    fn new_raw_round_trip() {
        for v in [0u64, 1, 2, 41, 42, 0x1_0000, u64::MAX - 1, u64::MAX] {
            assert_eq!(Epoch::new(v).raw(), v);
        }
    }

    #[test]
    fn display_is_bare_number() {
        assert_eq!(format!("{}", Epoch::ZERO), "0");
        assert_eq!(format!("{}", Epoch::new(42)), "42");
        assert_eq!(format!("{}", Epoch::new(u64::MAX)), format!("{}", u64::MAX));
    }
}
