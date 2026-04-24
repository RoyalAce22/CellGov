//! Half-open `[start, start + length)` byte range over the guest address space.

use crate::addr::GuestAddr;

/// Half-open byte range `[start, start + length)` in the guest address space.
///
/// Length rather than end address is stored so a zero-length range at a
/// particular start is unambiguously representable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ByteRange {
    start: GuestAddr,
    length: u64,
}

impl ByteRange {
    /// Construct a `ByteRange` of `length` bytes starting at `start`, or
    /// `None` if `start + length` would overflow `u64`.
    #[inline]
    pub const fn new(start: GuestAddr, length: u64) -> Option<Self> {
        match start.raw().checked_add(length) {
            Some(_) => Some(Self { start, length }),
            None => None,
        }
    }

    /// Inclusive lower bound.
    #[inline]
    pub const fn start(self) -> GuestAddr {
        self.start
    }

    /// Length in bytes.
    #[inline]
    pub const fn length(self) -> u64 {
        self.length
    }

    /// Exclusive upper bound. Infallible because `new` validated non-overflow.
    #[inline]
    pub const fn end(self) -> GuestAddr {
        GuestAddr::new(self.start.raw() + self.length)
    }

    /// Whether the range is zero bytes long.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.length == 0
    }

    /// Whether `addr` falls inside this range. Empty ranges contain nothing.
    #[inline]
    pub const fn contains_addr(self, addr: GuestAddr) -> bool {
        if self.length == 0 {
            return false;
        }
        let raw = addr.raw();
        raw >= self.start.raw() && raw < self.start.raw() + self.length
    }

    /// Whether this range and `other` share at least one byte.
    ///
    /// Any range involving an empty range returns false. Adjacent ranges
    /// (one ending exactly where the next begins) do not overlap.
    #[inline]
    pub const fn overlaps(self, other: ByteRange) -> bool {
        if self.length == 0 || other.length == 0 {
            return false;
        }
        let self_end = self.start.raw() + self.length;
        let other_end = other.start.raw() + other.length;
        self.start.raw() < other_end && other.start.raw() < self_end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(start: u64, length: u64) -> ByteRange {
        ByteRange::new(GuestAddr::new(start), length).expect("range fits")
    }

    #[test]
    fn construction_basic() {
        let br = r(0x1000, 0x80);
        assert_eq!(br.start(), GuestAddr::new(0x1000));
        assert_eq!(br.length(), 0x80);
        assert_eq!(br.end(), GuestAddr::new(0x1080));
        assert!(!br.is_empty());
    }

    #[test]
    fn empty_range_is_representable() {
        let br = r(0x1000, 0);
        assert!(br.is_empty());
        assert_eq!(br.start(), br.end());
    }

    #[test]
    fn construction_overflow_is_none() {
        let res = ByteRange::new(GuestAddr::new(u64::MAX), 1);
        assert_eq!(res, None);
    }

    #[test]
    fn construction_at_max_zero_length_is_ok() {
        let res = ByteRange::new(GuestAddr::new(u64::MAX), 0);
        assert!(res.is_some());
    }

    #[test]
    fn contains_addr_inside() {
        let br = r(0x100, 0x10);
        assert!(br.contains_addr(GuestAddr::new(0x100)));
        assert!(br.contains_addr(GuestAddr::new(0x108)));
        assert!(br.contains_addr(GuestAddr::new(0x10f)));
    }

    #[test]
    fn contains_addr_at_end_is_false() {
        let br = r(0x100, 0x10);
        assert!(!br.contains_addr(GuestAddr::new(0x110)));
    }

    #[test]
    fn contains_addr_below_is_false() {
        let br = r(0x100, 0x10);
        assert!(!br.contains_addr(GuestAddr::new(0xff)));
    }

    #[test]
    fn empty_range_contains_nothing() {
        let br = r(0x100, 0);
        assert!(!br.contains_addr(GuestAddr::new(0x100)));
    }

    #[test]
    fn overlap_overlapping_ranges() {
        let a = r(0x100, 0x20);
        let b = r(0x110, 0x20);
        assert!(a.overlaps(b));
        assert!(b.overlaps(a));
    }

    #[test]
    fn overlap_one_contains_other() {
        let outer = r(0x100, 0x100);
        let inner = r(0x140, 0x10);
        assert!(outer.overlaps(inner));
        assert!(inner.overlaps(outer));
    }

    #[test]
    fn overlap_identical_ranges() {
        let a = r(0x100, 0x20);
        assert!(a.overlaps(a));
    }

    #[test]
    fn overlap_adjacent_is_false() {
        let a = r(0x100, 0x10);
        let b = r(0x110, 0x10);
        assert!(!a.overlaps(b));
        assert!(!b.overlaps(a));
    }

    #[test]
    fn overlap_disjoint_is_false() {
        let a = r(0x100, 0x10);
        let b = r(0x200, 0x10);
        assert!(!a.overlaps(b));
    }

    #[test]
    fn overlap_with_empty_is_false() {
        let a = r(0x100, 0x10);
        let empty = r(0x108, 0);
        assert!(!a.overlaps(empty));
        assert!(!empty.overlaps(a));
    }

    #[test]
    fn overlap_two_empty_is_false() {
        let a = r(0x100, 0);
        let b = r(0x100, 0);
        assert!(!a.overlaps(b));
    }
}
