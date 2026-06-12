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

    /// Construct a `ByteRange` for a 32-bit guest address plus a u32
    /// length. Infallible: `u32::MAX + u32::MAX < u64::MAX`, so the
    /// overflow path `new` guards against is unreachable here.
    ///
    /// Use this in dispatch handlers that source their pointer from a
    /// classified `Lv2Request` u32 slot -- it removes the option/expect
    /// pair the call site would otherwise carry.
    #[inline]
    pub const fn contiguous_u32(addr: u32, len: u32) -> Self {
        Self {
            start: GuestAddr::new(addr as u64),
            length: len as u64,
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

    /// Exclusive end as `u64`. The `new` precondition guarantees the
    /// addition does not overflow; `wrapping_add` keeps a future bug
    /// here a wrong number rather than UB in `const` contexts, and the
    /// `debug_assert` keeps tests honest.
    #[inline]
    const fn end_raw(self) -> u64 {
        debug_assert!(self.start.raw().checked_add(self.length).is_some());
        self.start.raw().wrapping_add(self.length)
    }

    /// Exclusive upper bound. Infallible because `new` validated non-overflow.
    #[inline]
    pub const fn end(self) -> GuestAddr {
        GuestAddr::new(self.end_raw())
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
        raw >= self.start.raw() && raw < self.end_raw()
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
        self.start.raw() < other.end_raw() && other.start.raw() < self.end_raw()
    }
}

#[cfg(test)]
#[path = "tests/range_tests.rs"]
mod tests;
