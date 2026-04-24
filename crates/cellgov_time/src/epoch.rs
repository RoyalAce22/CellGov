//! Commit-batch counter used as the granularity for state hashes.

/// A commit-batch counter; advances once per closed batch.
///
/// Within one epoch the set of committed effects is closed: all writes in
/// the batch become visible together, or none do. State hashes are taken
/// only at epoch boundaries.
///
/// Distinct from [`crate::ticks::GuestTicks`] and [`crate::budget::Budget`];
/// the three do not implicitly convert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Epoch(u64);

impl Epoch {
    /// Starting epoch before any commit has run.
    pub const ZERO: Self = Self(0);

    /// Lift a raw count into an `Epoch`.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Underlying epoch number.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Successor epoch, or `None` on overflow.
    ///
    /// Overflow is an invariant violation; callers must not silently wrap.
    #[inline]
    pub const fn next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(v) => Some(Self(v)),
            None => None,
        }
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
    fn next_at_max_is_none() {
        assert_eq!(Epoch::new(u64::MAX).next(), None);
    }

    #[test]
    fn ordering_is_total_and_monotonic() {
        assert!(Epoch::new(0) < Epoch::new(1));
        assert!(Epoch::new(100) > Epoch::new(99));
    }
}
