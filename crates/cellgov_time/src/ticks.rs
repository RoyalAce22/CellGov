//! The runtime's monotonic ordering clock for guest-visible events.

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
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, derive_more::Display,
)]
#[display("{_0}")]
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

#[cfg(test)]
#[path = "tests/ticks_tests.rs"]
mod tests;
