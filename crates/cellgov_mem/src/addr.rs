//! Guest address newtype.
//!
//! `GuestAddr` is the runtime's representation of a location in the guest's
//! flat address space. It is intentionally a distinct type from a host
//! pointer or a bare `u64`: every site that produces an address must be
//! deliberate, since address arithmetic is the easiest place to introduce
//! aliasing and overflow bugs that are invisible until they corrupt
//! committed memory.
//!
//! There are no `From<u64>` or `Into<u64>` impls on purpose. Use
//! [`GuestAddr::new`] to lift, [`GuestAddr::raw`] to lower.

/// A location in the guest's flat address space, in bytes from address zero.
///
/// `GuestAddr` is `Copy + Ord + Hash`. It does not implement `Add<u64>`
/// because silent overflow on address arithmetic would be a determinism
/// hazard: out-of-range writes are validation failures the runtime must
/// surface as faults, not wrap around to a different region. Use
/// [`GuestAddr::checked_offset`] for the explicit, fallible form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct GuestAddr(u64);

impl GuestAddr {
    /// The lowest possible guest address. Mostly useful as a default
    /// in tests and as a sentinel for "no address yet".
    pub const ZERO: Self = Self(0);

    /// Lift a raw `u64` into the guest address space.
    ///
    /// There is no `From<u64>` impl: every lift site should be visible
    /// at the call graph so the runtime can audit who is fabricating
    /// addresses out of integer literals.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Return the underlying byte offset. Use sparingly; prefer ordering
    /// comparisons or [`GuestAddr::checked_offset`] for arithmetic.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Advance this address by `bytes`, returning `None` on overflow.
    ///
    /// Overflow here is a real failure mode: a guest may legitimately
    /// compute an address near `u64::MAX`, and silently wrapping would
    /// alias the low end of the address space. Callers must handle the
    /// `None` case explicitly.
    #[inline]
    pub const fn checked_offset(self, bytes: u64) -> Option<Self> {
        match self.0.checked_add(bytes) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }

    /// Distance in bytes from `earlier` to `self`, or `None` if `earlier`
    /// is above `self`. Useful for computing the length of a range whose
    /// endpoints are known but whose direction is not statically obvious.
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
