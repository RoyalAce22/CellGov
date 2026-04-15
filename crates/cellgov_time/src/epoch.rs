//! Logical epoch -- advances at commit boundaries.
//!
//! An epoch tick marks a point at which the set of committed effects is
//! closed and visible. Epoch is the granularity at which determinism
//! comparisons -- state hashes, replay checkpoints -- are taken.
//!
//! `Epoch` is a distinct type from [`crate::ticks::GuestTicks`] and
//! [`crate::budget::Budget`]: the three concepts must not implicitly convert.

/// A logical epoch counter. Advances exactly once per commit batch.
///
/// Within a single epoch the set of committed effects is closed: all writes
/// in a batch become visible together, or none do. Epoch boundaries are the
/// only points at which the runtime takes state hashes for replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Epoch(u64);

impl Epoch {
    /// The first epoch. Every runtime starts here before any commits.
    pub const ZERO: Self = Self(0);

    /// Construct an `Epoch` from a raw count.
    ///
    /// There is no `From<u64>` impl: epoch values are produced by the
    /// commit pipeline, not by ad-hoc arithmetic at call sites.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Return the underlying epoch number. Use sparingly -- prefer ordering
    /// and `next` over arithmetic on raw values.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// The successor epoch, or `None` on overflow.
    ///
    /// Overflow is a runtime invariant violation, not a recoverable
    /// condition; callers must handle it explicitly rather than wrapping.
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
