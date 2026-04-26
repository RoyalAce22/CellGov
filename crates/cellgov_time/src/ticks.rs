//! The runtime's monotonic ordering clock for guest-visible events.

use core::fmt;

/// A point in guest virtual time, in ticks since runtime start.
///
/// Totally ordered, monotonically non-decreasing, never derived from host
/// time.
///
/// ```compile_fail
/// use cellgov_time::{Budget, GuestTicks};
/// let _: Budget = GuestTicks::ZERO.into();
/// ```
///
/// ```compile_fail
/// use cellgov_time::{Budget, GuestTicks};
/// let _: GuestTicks = Budget::ZERO.into();
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct GuestTicks(u64);

impl GuestTicks {
    /// Origin of guest time.
    pub const ZERO: Self = Self(0);

    /// Lift a raw count into a `GuestTicks`.
    #[doc(hidden)]
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Underlying tick count.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Advance by `delta` ticks, `None` on overflow.
    #[must_use = "checked_add returns the advanced value; assign or pattern-match it"]
    #[inline]
    pub const fn checked_add(self, delta: GuestTicks) -> Option<Self> {
        match self.0.checked_add(delta.0) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }

    /// Advance by `delta` ticks, saturating at `u64::MAX`.
    #[must_use = "saturating_add returns the advanced value; assign or pattern-match it"]
    #[inline]
    pub const fn saturating_add(self, delta: GuestTicks) -> Self {
        Self(self.0.saturating_add(delta.0))
    }

    /// Ticks elapsed from `earlier` to `self`, or `None` if `earlier` is in
    /// the future.
    ///
    /// Guest time never moves backward, so `None` here is an invariant
    /// violation at the call site.
    #[must_use = "checked_duration_since returns the duration; assign or pattern-match it"]
    #[inline]
    pub const fn checked_duration_since(self, earlier: GuestTicks) -> Option<GuestTicks> {
        match self.0.checked_sub(earlier.0) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }
}

impl fmt::Display for GuestTicks {
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
    fn ordering_holds_at_max_boundary() {
        assert!(GuestTicks::new(u64::MAX - 1) < GuestTicks::new(u64::MAX));
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

    #[test]
    fn new_raw_round_trip() {
        for v in [0u64, 1, 2, 41, 42, 0x1_0000, u64::MAX - 1, u64::MAX] {
            assert_eq!(GuestTicks::new(v).raw(), v);
        }
    }

    #[test]
    fn default_is_zero() {
        assert_eq!(GuestTicks::default(), GuestTicks::ZERO);
    }

    #[test]
    fn display_is_bare_number() {
        assert_eq!(format!("{}", GuestTicks::ZERO), "0");
        assert_eq!(format!("{}", GuestTicks::new(42)), "42");
        assert_eq!(
            format!("{}", GuestTicks::new(u64::MAX)),
            format!("{}", u64::MAX),
        );
    }
}
