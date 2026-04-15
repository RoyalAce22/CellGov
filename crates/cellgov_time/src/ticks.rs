//! Guest ticks -- the authoritative monotonic ordering clock.
//!
//! `GuestTicks` is the runtime-wide clock used to order every guest-visible
//! event. It must never move backward, must never be derived from host time,
//! and must never be implicitly convertible to or from `Budget` or `Epoch`.
//! Conversions across these types are scheduler policy and live elsewhere.

/// A point in guest virtual time, measured in ticks since runtime start.
///
/// `GuestTicks` is a totally ordered, monotonically non-decreasing counter.
/// It is the authoritative ordering clock for every guest-visible event in
/// the runtime. It is a distinct type from [`crate::budget::Budget`] and
/// [`crate::epoch::Epoch`]: the three concepts must not implicitly convert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct GuestTicks(u64);

impl GuestTicks {
    /// The zero point of guest time. This is the only sanctioned origin.
    pub const ZERO: Self = Self(0);

    /// Construct a `GuestTicks` from a raw count.
    ///
    /// This is the only way to lift a `u64` into guest time. There is no
    /// `From<u64>` impl: every site that produces a tick value must be
    /// explicit so the time domain is auditable from the call graph.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Return the underlying tick count. Use sparingly -- prefer ordering
    /// comparisons against other `GuestTicks` values.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Advance by `delta` ticks, returning `None` on overflow.
    ///
    /// Guest time is monotonic, so the only failure mode is overflow of the
    /// underlying counter. Callers must handle this explicitly; there is no
    /// silent wraparound.
    #[inline]
    pub const fn checked_add(self, delta: GuestTicks) -> Option<Self> {
        match self.0.checked_add(delta.0) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }

    /// Advance by `delta` ticks, saturating at `u64::MAX`.
    ///
    /// Saturation is a defined, deterministic behavior; it does not depend on
    /// host state. Use this only when overflow truly is unreachable in the
    /// runtime's design and you want a total operation.
    #[inline]
    pub const fn saturating_add(self, delta: GuestTicks) -> Self {
        Self(self.0.saturating_add(delta.0))
    }

    /// Compute the elapsed ticks from `earlier` to `self`, or `None` if
    /// `earlier` is in the future. Guest time never moves backward, so a
    /// `None` here is a runtime invariant violation at the call site.
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
