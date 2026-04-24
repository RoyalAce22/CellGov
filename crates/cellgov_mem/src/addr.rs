//! Guest address newtype.
//!
//! No `From<u64>`/`Into<u64>` impls: every lift site is explicit via
//! [`GuestAddr::new`] / [`GuestAddr::raw`].

/// A location in the guest's flat address space, in bytes from zero.
///
/// No `Add<u64>`: overflow on address arithmetic must surface as a fault
/// rather than wrap. Use [`GuestAddr::checked_offset`] for the fallible form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct GuestAddr(u64);

impl GuestAddr {
    /// The lowest possible guest address.
    pub const ZERO: Self = Self(0);

    /// Lift a raw `u64` into the guest address space.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Return the underlying byte offset.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Advance this address by `bytes`, returning `None` on overflow.
    #[inline]
    pub const fn checked_offset(self, bytes: u64) -> Option<Self> {
        match self.0.checked_add(bytes) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }

    /// Distance in bytes from `earlier` to `self`, or `None` if `earlier > self`.
    #[inline]
    pub const fn checked_distance_from(self, earlier: GuestAddr) -> Option<u64> {
        self.0.checked_sub(earlier.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_is_origin() {
        assert_eq!(GuestAddr::ZERO, GuestAddr::new(0));
        assert_eq!(GuestAddr::default(), GuestAddr::ZERO);
    }

    #[test]
    fn roundtrip() {
        assert_eq!(GuestAddr::new(0xdead_beef).raw(), 0xdead_beef);
    }

    #[test]
    fn ordering_is_total() {
        assert!(GuestAddr::new(1) < GuestAddr::new(2));
        assert_eq!(GuestAddr::new(7), GuestAddr::new(7));
    }

    #[test]
    fn checked_offset_within_range() {
        let a = GuestAddr::new(0x1000);
        assert_eq!(a.checked_offset(0x10), Some(GuestAddr::new(0x1010)));
    }

    #[test]
    fn checked_offset_at_zero() {
        assert_eq!(GuestAddr::ZERO.checked_offset(0), Some(GuestAddr::ZERO));
    }

    #[test]
    fn checked_offset_overflow_is_none() {
        let a = GuestAddr::new(u64::MAX);
        assert_eq!(a.checked_offset(1), None);
    }

    #[test]
    fn checked_distance_from_earlier() {
        let lo = GuestAddr::new(0x100);
        let hi = GuestAddr::new(0x180);
        assert_eq!(hi.checked_distance_from(lo), Some(0x80));
    }

    #[test]
    fn checked_distance_from_self_is_zero() {
        let a = GuestAddr::new(0x42);
        assert_eq!(a.checked_distance_from(a), Some(0));
    }

    #[test]
    fn checked_distance_from_above_is_none() {
        let lo = GuestAddr::new(0x100);
        let hi = GuestAddr::new(0x180);
        assert_eq!(lo.checked_distance_from(hi), None);
    }
}
