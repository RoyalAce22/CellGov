//! Per-unit private memory (e.g. an SPU's 256 KiB local store).
//!
//! Writes do not flow through the commit pipeline: a `LocalStore` is only
//! observable by its owning unit, so immediate mutation is determinism-safe.
//! Transfers to/from [`crate::guest::GuestMemory`] happen through DMA effects,
//! not through direct sharing.

use crate::guest::MemError;
use crate::range::ByteRange;

/// A unit-private flat byte region. Out-of-range operations return
/// `None`/`Err` rather than panic.
#[derive(Debug, Clone)]
pub struct LocalStore {
    bytes: Vec<u8>,
}

impl LocalStore {
    /// Construct a fresh `LocalStore` of `size` bytes, zero-initialized.
    #[inline]
    pub fn new(size: usize) -> Self {
        Self {
            bytes: vec![0u8; size],
        }
    }

    /// Total local store size in bytes.
    #[inline]
    pub fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    /// Read the bytes covered by `range`, or `None` if it extends past `size()`.
    ///
    /// A zero-length range at any in-bounds start (including exactly `size()`)
    /// returns an empty slice.
    pub fn read(&self, range: ByteRange) -> Option<&[u8]> {
        let start = range.start().raw();
        let end = start.checked_add(range.length())?;
        let size = self.size();
        if end > size {
            return None;
        }
        let start_usize = usize::try_from(start).ok()?;
        let end_usize = usize::try_from(end).ok()?;
        Some(&self.bytes[start_usize..end_usize])
    }

    /// Write `bytes` to `range`. Not gated by the commit pipeline: the owning
    /// unit may call this freely.
    ///
    /// # Errors
    ///
    /// - [`MemError::LengthMismatch`] if `bytes.len() as u64 != range.length()`.
    /// - [`MemError::OutOfRange`] if the range extends past `size()`.
    pub fn write(&mut self, range: ByteRange, bytes: &[u8]) -> Result<(), MemError> {
        if bytes.len() as u64 != range.length() {
            return Err(MemError::LengthMismatch);
        }
        let start = range.start().raw();
        let end = start
            .checked_add(range.length())
            .ok_or(MemError::OutOfRange)?;
        let size = self.size();
        if end > size {
            return Err(MemError::OutOfRange);
        }
        let start_usize = usize::try_from(start).map_err(|_| MemError::OutOfRange)?;
        let end_usize = usize::try_from(end).map_err(|_| MemError::OutOfRange)?;
        self.bytes[start_usize..end_usize].copy_from_slice(bytes);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::addr::GuestAddr;

    fn range(start: u64, length: u64) -> ByteRange {
        ByteRange::new(GuestAddr::new(start), length).unwrap()
    }

    #[test]
    fn new_is_zero_initialized() {
        let ls = LocalStore::new(16);
        assert_eq!(ls.size(), 16);
        assert_eq!(ls.read(range(0, 16)).unwrap(), &[0u8; 16]);
    }

    #[test]
    fn write_then_read() {
        let mut ls = LocalStore::new(8);
        ls.write(range(2, 4), &[1, 2, 3, 4]).unwrap();
        assert_eq!(ls.read(range(2, 4)).unwrap(), &[1, 2, 3, 4]);
        assert_eq!(ls.read(range(0, 2)).unwrap(), &[0, 0]);
        assert_eq!(ls.read(range(6, 2)).unwrap(), &[0, 0]);
    }

    #[test]
    fn read_zero_length_at_in_bounds_start() {
        let ls = LocalStore::new(16);
        let s = ls.read(range(8, 0)).unwrap();
        assert!(s.is_empty());
    }

    #[test]
    fn read_zero_length_at_size_boundary() {
        let ls = LocalStore::new(16);
        let s = ls.read(range(16, 0)).unwrap();
        assert!(s.is_empty());
    }

    #[test]
    fn read_past_end_is_none() {
        let ls = LocalStore::new(16);
        assert_eq!(ls.read(range(15, 2)), None);
    }

    #[test]
    fn read_starting_past_end_is_none() {
        let ls = LocalStore::new(16);
        assert_eq!(ls.read(range(17, 1)), None);
    }

    #[test]
    fn write_length_mismatch_rejected() {
        let mut ls = LocalStore::new(8);
        let err = ls.write(range(0, 4), &[1, 2, 3]).unwrap_err();
        assert_eq!(err, MemError::LengthMismatch);
        assert_eq!(ls.read(range(0, 4)).unwrap(), &[0, 0, 0, 0]);
    }

    #[test]
    fn write_out_of_range_rejected() {
        let mut ls = LocalStore::new(8);
        let err = ls.write(range(6, 4), &[1, 2, 3, 4]).unwrap_err();
        assert_eq!(err, MemError::OutOfRange);
        assert_eq!(ls.read(range(0, 8)).unwrap(), &[0; 8]);
    }

    #[test]
    fn write_zero_length_is_noop() {
        let mut ls = LocalStore::new(8);
        ls.write(range(4, 0), &[]).unwrap();
        assert_eq!(ls.read(range(0, 8)).unwrap(), &[0; 8]);
    }

    #[test]
    fn overlapping_writes_apply_in_call_order() {
        let mut ls = LocalStore::new(8);
        ls.write(range(0, 4), &[1, 1, 1, 1]).unwrap();
        ls.write(range(2, 4), &[2, 2, 2, 2]).unwrap();
        assert_eq!(ls.read(range(0, 8)).unwrap(), &[1, 1, 2, 2, 2, 2, 0, 0]);
    }

    #[test]
    fn clone_is_independent() {
        let mut a = LocalStore::new(4);
        a.write(range(0, 4), &[9, 9, 9, 9]).unwrap();
        let b = a.clone();
        a.write(range(0, 4), &[0, 0, 0, 0]).unwrap();
        assert_eq!(a.read(range(0, 4)).unwrap(), &[0, 0, 0, 0]);
        assert_eq!(b.read(range(0, 4)).unwrap(), &[9, 9, 9, 9]);
    }
}
