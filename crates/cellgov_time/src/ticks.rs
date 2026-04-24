//! The runtime's monotonic ordering clock for guest-visible events.

/// A point in guest virtual time, in ticks since runtime start.
///
/// Totally ordered, monotonically non-decreasing, never derived from host
/// time. Distinct from [`crate::budget::Budget`] and [`crate::epoch::Epoch`];
/// the three do not implicitly convert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct GuestTicks(u64);

impl GuestTicks {
    /// Origin of guest time.
    pub const ZERO: Self = Self(0);

    /// Lift a raw count into a `GuestTicks`.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Underlying tick count.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Advance by `delta` ticks, returning `None` on overflow.
    ///
    /// Overflow is the only failure mode; there is no silent wraparound.
    #[inline]
    pub const fn checked_add(self, delta: GuestTicks) -> Option<Self> {
        match self.0.checked_add(delta.0) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }

    /// Advance by `delta` ticks, saturating at `u64::MAX`.
    #[inline]
    pub const fn saturating_add(self, delta: GuestTicks) -> Self {
        Self(self.0.saturating_add(delta.0))
    }

    /// Ticks elapsed from `earlier` to `self`, or `None` if `earlier` is in
    /// the future.
    ///
    /// Guest time never moves backward, so `None` here is an invariant
    /// violation at the call site.
    #[inline]
    pub const fn checked_duration_since(self, earlier: GuestTicks) -> Option<GuestTicks> {
        match self.0.checked_sub(earlier.0) {
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
        assert_eq!(GuestTicks::ZERO.raw(), 0);
        assert_eq!(GuestTicks::ZERO, GuestTicks::new(0));
    }

    #[test]
    fn ordering_is_total_and_monotonic() {
        let a = GuestTicks::new(5);
        let b = GuestTicks::new(10);
        assert!(a < b);
        assert!(b > a);
        assert_eq!(a, GuestTicks::new(5));
    }

    #[test]
    fn checked_add_advances() {
        let t = GuestTicks::new(100);
        assert_eq!(
            t.checked_add(GuestTicks::new(25)),
            Some(GuestTicks::new(125))
        );
    }

    #[test]
    fn checked_add_overflows_to_none() {
        let t = GuestTicks::new(u64::MAX);
        assert_eq!(t.checked_add(GuestTicks::new(1)), None);
    }

    #[test]
    fn saturating_add_pins_at_max() {
        let t = GuestTicks::new(u64::MAX - 3);
        assert_eq!(
            t.saturating_add(GuestTicks::new(10)),
            GuestTicks::new(u64::MAX)
        );
    }

    #[test]
    fn duration_since_earlier_is_some() {
        let earlier = GuestTicks::new(7);
        let later = GuestTicks::new(20);
        assert_eq!(
            later.checked_duration_since(earlier),
            Some(GuestTicks::new(13))
        );
    }

    #[test]
    fn duration_since_future_is_none() {
        let earlier = GuestTicks::new(20);
        let later = GuestTicks::new(7);
        assert_eq!(later.checked_duration_since(earlier), None);
    }
}
