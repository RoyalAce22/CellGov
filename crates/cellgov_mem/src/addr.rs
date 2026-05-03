//! Guest address newtype.
//!
//! No `From<u64>`/`Into<u64>` impls and no `Default`: every lift site is
//! explicit via [`GuestAddr::new`] / [`GuestAddr::raw`], and a default-
//! constructed `GuestAddr` would point at `0x0` -- a real, mapped PS3
//! address, not a sentinel.

use std::fmt;

/// A location in the guest's flat address space, in bytes from zero.
///
/// No `Add<u64>`: overflow on address arithmetic must surface as a fault
/// rather than wrap. Use [`GuestAddr::checked_offset`] for the fallible
/// signed form (PPC ISA v2.02 Book I sec. 1.12.2: D-form / DS-form / indexed
/// EA arithmetic is 64-bit two's-complement with a signed displacement).
///
/// `repr(transparent)` so the type is ABI-identical to the underlying
/// `u64`. `Option<GuestAddr>` is still 16 bytes -- `u64` has no niche.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
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

    /// Advance this address by a signed byte displacement, as PowerPC
    /// D-form / DS-form / indexed Storage Access instructions do.
    /// Returns `None` on overflow above `u64::MAX` or underflow below 0.
    ///
    /// The only architectural ceiling enforced here is `u64::MAX`. Per CBE
    /// Programming Handbook v1.1 sec. 4.2.9 (p. 101), EA arithmetic is
    /// always 64-bit two's-complement -- "the 64-bit EA is first calculated
    /// as usual" -- regardless of `MSR\[SF\]`. The 32-bit-mode high-bits-cleared
    /// behavior is a property of memory access (truncation at the MMU), not
    /// of address computation. Tighter ceilings (32-bit MMU truncation,
    /// region-map bounds, Cell's 2^42 RA limit per Figure 4-4 p. 100)
    /// belong to the layer that performs the access, not to this type.
    #[inline]
    pub const fn checked_offset(self, displacement: i64) -> Option<Self> {
        if displacement >= 0 {
            match self.0.checked_add(displacement as u64) {
                Some(v) => Some(Self(v)),
                None => None,
            }
        } else {
            // unsigned_abs handles i64::MIN cleanly (yields 2^63 in u64).
            match self.0.checked_sub(displacement.unsigned_abs()) {
                Some(v) => Some(Self(v)),
                None => None,
            }
        }
    }

    /// Distance in bytes from `earlier` to `self`, or `None` if `earlier > self`.
    #[inline]
    pub const fn checked_distance_from(self, earlier: GuestAddr) -> Option<u64> {
        self.0.checked_sub(earlier.0)
    }
}

impl fmt::Display for GuestAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:016x}", self.0)
    }
}

impl fmt::LowerHex for GuestAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::LowerHex::fmt(&self.0, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_is_origin() {
        assert_eq!(GuestAddr::ZERO, GuestAddr::new(0));
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
    fn checked_offset_identity_from_nonzero() {
        let a = GuestAddr::new(0xdead_beef);
        assert_eq!(a.checked_offset(0), Some(a));
    }

    #[test]
    fn checked_offset_negative_displacement() {
        // PPC D-form `lwz r3, -0x10(r1)` style: signed displacement.
        let a = GuestAddr::new(0x1000);
        assert_eq!(a.checked_offset(-0x10), Some(GuestAddr::new(0xFF0)));
    }

    #[test]
    fn checked_offset_underflow_below_zero_is_none() {
        assert_eq!(GuestAddr::ZERO.checked_offset(-1), None);
        assert_eq!(GuestAddr::new(0x10).checked_offset(-0x11), None);
    }

    #[test]
    fn checked_offset_overflow_above_u64_max_is_none() {
        let a = GuestAddr::new(u64::MAX);
        assert_eq!(a.checked_offset(1), None);
    }

    #[test]
    fn checked_offset_handles_high_address_with_positive_displacement() {
        // 0xFFFF_FFFF_FFFF_FFF0 doesn't fit in i64 (it's negative when
        // bit-cast). The checked_offset must still advance it forward
        // by a small positive displacement without claiming overflow.
        let high = GuestAddr::new(0xFFFF_FFFF_FFFF_FFF0);
        assert_eq!(
            high.checked_offset(0x0F),
            Some(GuestAddr::new(0xFFFF_FFFF_FFFF_FFFF))
        );
        // +0x10 from ...FFF0 lands at 2^64, which doesn't fit in u64.
        assert_eq!(high.checked_offset(0x10), None);
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

    #[test]
    fn display_is_hex_with_0x_prefix_and_16_digit_width() {
        assert_eq!(format!("{}", GuestAddr::ZERO), "0x0000000000000000");
        assert_eq!(
            format!("{}", GuestAddr::new(0xC000_0040)),
            "0x00000000c0000040"
        );
    }

    #[test]
    fn lower_hex_format_works_without_default_width() {
        assert_eq!(format!("{:x}", GuestAddr::new(0xC000_0040)), "c0000040");
        assert_eq!(format!("{:08x}", GuestAddr::new(0xC0)), "000000c0");
    }

    #[test]
    fn repr_transparent_keeps_size_at_eight_bytes() {
        // repr(transparent) is what guarantees this; without it the type
        // would still happen to be 8 bytes today, but the layout would
        // not be a documented contract.
        assert_eq!(core::mem::size_of::<GuestAddr>(), 8);
        assert_eq!(
            core::mem::align_of::<GuestAddr>(),
            core::mem::align_of::<u64>()
        );
    }

    #[test]
    fn is_send_and_sync() {
        // Tripwire against a future field that isn't `Send + Sync`
        // (e.g., an `Rc<&str>` label) silently breaking commit-pipeline
        // hand-offs.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<GuestAddr>();
    }
}
