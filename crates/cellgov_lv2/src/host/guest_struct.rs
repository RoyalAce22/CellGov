//! Big-endian field reads over guest structs, shared by the LV2 handlers.

use crate::host::Lv2Runtime;

/// A guest struct's bytes, read at big-endian field offsets.
///
/// # Panics
///
/// Every accessor panics when the field runs past the end of the
/// wrapped slice.
#[derive(Clone, Copy)]
pub(crate) struct GuestStruct<'a> {
    bytes: &'a [u8],
}

impl<'a> GuestStruct<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    /// The `len` bytes at `addr`, or `None` when the range is unmapped.
    ///
    /// # Panics
    ///
    /// Debug builds panic when the runtime breaks the `read_committed`
    /// contract and answers `Some` with fewer than `len` bytes. Callers
    /// use this read as the gate that `len` bytes are mapped. A short
    /// answer passes that gate on too few bytes.
    pub(crate) fn read(rt: &'a dyn Lv2Runtime, addr: u64, len: usize) -> Option<Self> {
        let bytes = rt.read_committed(addr, len)?;
        debug_assert_eq!(
            bytes.len(),
            len,
            "Lv2Runtime::read_committed: Some(bytes) must carry exactly len bytes"
        );
        Some(Self::new(bytes))
    }

    pub(crate) fn u16_at(&self, off: usize) -> u16 {
        u16::from_be_bytes([self.bytes[off], self.bytes[off + 1]])
    }

    pub(crate) fn u32_at(&self, off: usize) -> u32 {
        u32::from_be_bytes([
            self.bytes[off],
            self.bytes[off + 1],
            self.bytes[off + 2],
            self.bytes[off + 3],
        ])
    }

    pub(crate) fn u64_at(&self, off: usize) -> u64 {
        u64::from_be_bytes([
            self.bytes[off],
            self.bytes[off + 1],
            self.bytes[off + 2],
            self.bytes[off + 3],
            self.bytes[off + 4],
            self.bytes[off + 5],
            self.bytes[off + 6],
            self.bytes[off + 7],
        ])
    }
}

/// Big-endian `u32` at `addr`, or `None` when the range is unmapped.
pub(crate) fn read_be_u32(rt: &dyn Lv2Runtime, addr: u64) -> Option<u32> {
    GuestStruct::read(rt, addr, 4).map(|s| s.u32_at(0))
}

/// Big-endian `u64` at `addr`, or `None` when the range is unmapped.
pub(crate) fn read_be_u64(rt: &dyn Lv2Runtime, addr: u64) -> Option<u64> {
    GuestStruct::read(rt, addr, 8).map(|s| s.u64_at(0))
}

#[cfg(test)]
#[path = "tests/guest_struct_tests.rs"]
mod tests;
