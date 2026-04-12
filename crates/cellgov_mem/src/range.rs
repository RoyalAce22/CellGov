//! Byte range over the guest address space.
//!
//! `ByteRange` is a half-open `[start, start + length)` interval used
//! everywhere the runtime needs to talk about a contiguous span of guest
//! memory: `SharedWriteIntent` payloads, `DmaRequest` source/destination
//! windows, page descriptors, and conflict-detection between staged
//! writes. It is intentionally a small value type with no allocation and
//! no host references.

use crate::addr::GuestAddr;

/// A half-open byte range `[start, start + length)` in the guest address
/// space.
///
/// Length is stored separately rather than as an end address so that a
/// zero-length range is representable without ambiguity (`start == end`
/// would otherwise be indistinguishable from "the empty range that begins
/// at `start`"). Construction is fallible: a range whose end would
/// overflow `u64` is rejected by [`ByteRange::new`] rather than silently
/// wrapping into the low end of the address space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ByteRange {
    start: GuestAddr,
    length: u64,
}

impl ByteRange {
    /// Construct a `ByteRange` of `length` bytes starting at `start`.
    ///
    /// Returns `None` if `start + length` would overflow `u64`. A
    /// zero-length range is allowed and is the canonical "empty" range
    /// at a particular address.
    #[inline]
    pub const fn new(start: GuestAddr, length: u64) -> Option<Self> {
        match start.raw().checked_add(length) {
            Some(_) => Some(Self { start, length }),
            None => None,
        }
    }

    /// The inclusive lower bound of the range.
    #[inline]
    pub const fn start(self) -> GuestAddr {
        self.start
    }

    /// The length of the range in bytes.
    #[inline]
    pub const fn length(self) -> u64 {
        self.length
    }

    /// The exclusive upper bound of the range. Constructing the range
    /// already verified this does not overflow, so this is infallible.
    #[inline]
    pub const fn end(self) -> GuestAddr {
        // Safe: validated at construction.
        GuestAddr::new(self.start.raw() + self.length)
    }

    /// Whether the range contains zero bytes.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.length == 0
    }

    /// Whether `addr` falls inside this range. Empty ranges contain
    /// nothing, even their own start address.
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
    /// Two empty ranges never overlap. A non-empty range never overlaps
    /// an empty range. Two non-empty ranges overlap iff their half-open
    /// intervals intersect: `self.start < other.end && other.start <
    /// self.end`. Adjacent ranges (one ending exactly where the next
    /// begins) do not overlap, by design -- contiguous-but-disjoint is
    /// the correct answer for write conflict detection.
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
        // [0x100, 0x110) and [0x110, 0x120) share no byte.
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
