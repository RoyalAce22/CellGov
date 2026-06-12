//! Guest address newtype.
//!
//! No `From<u64>`/`Into<u64>` impls and no `Default`: every lift site is
//! explicit via [`GuestAddr::new`] / [`GuestAddr::raw`], and a default-
//! constructed `GuestAddr` would point at `0x0` -- a real, mapped PS3
//! address, not a sentinel.

/// A location in the guest's flat address space, in bytes from zero.
///
/// No `Add<u64>`: overflow on address arithmetic must surface as a
/// fault rather than wrap. Use [`GuestAddr::checked_offset`] for the
/// fallible signed form.
// [PPC-Book1 p:15 s:1.12 Storage Addressing] D/DS-form and indexed EAs use 64-bit two's-complement signed-displacement arithmetic.
///
/// `repr(transparent)` so the type is ABI-identical to the underlying
/// `u64`. `Option<GuestAddr>` is still 16 bytes -- `u64` has no niche.
///
/// Serde shape: bare JSON number (`#[serde(transparent)]`). On-disk
/// addresses round-trip as raw `u64`s, not nested objects, so the
/// wire format matches what `--checkpoint pc=0xADDR` accepts.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    derive_more::Display,
    derive_more::LowerHex,
)]
#[repr(transparent)]
#[serde(transparent)]
#[display("0x{_0:016x}")]
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
    /// The only architectural ceiling enforced here is `u64::MAX`. Per
    /// [CBE-Handbook p:101 s:4.2.9], EA arithmetic is always 64-bit
    /// two's-complement regardless of `MSR\[SF\]`. The 32-bit-mode
    /// high-bits-cleared behavior is a property of memory access
    /// (truncation at the MMU), not of address computation. Tighter
    /// ceilings (32-bit MMU truncation, region-map bounds, Cell's 2^42
    /// RA limit per [CBE-Handbook p:100]) belong to the layer that
    /// performs the access, not to this type.
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

#[cfg(test)]
#[path = "tests/addr_tests.rs"]
mod tests;
