//! Barrier identifier.
//!
//! Currently only the leaf identifier lives here. The barrier state
//! machine (participant count, waiting set, release logic) requires
//! scheduler integration to wake blocked participants on release.
//! That is Phase 2 sync work; until then, `WaitOnEvent` with a
//! `WaitTarget::Barrier` unconditionally blocks the issuing unit.
//!
//! `BarrierId` is the handle a unit references when it raises an
//! `Effect::WaitOnEvent` against a barrier.

/// A stable identifier for a barrier (or barrier-shaped sync primitive)
/// in the runtime.
///
/// Barriers, mutexes, and semaphores share this id type because they
/// share the same wait-and-release shape. The state machine
/// behind the id is what differs; the runtime's identification scheme
/// does not. There is no `From<u64>` impl on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BarrierId(u64);

impl BarrierId {
    /// Construct a `BarrierId` from a raw value.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Return the underlying id value.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        assert_eq!(BarrierId::new(13).raw(), 13);
    }

    #[test]
    fn default_is_zero() {
        assert_eq!(BarrierId::default(), BarrierId::new(0));
    }

    #[test]
    fn ordering_is_total() {
        assert!(BarrierId::new(8) < BarrierId::new(9));
        assert_eq!(BarrierId::new(2), BarrierId::new(2));
    }
}
