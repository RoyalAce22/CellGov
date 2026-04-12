//! `LocalStore` -- per-unit private memory.
//!
//! `LocalStore` is the third of the three memory layers, alongside
//! [`crate::guest::GuestMemory`] and
//! [`crate::staging::StagingMemory`]. It models the unit-private
//! memory regions that exist on Cell-style architectures: an SPU's
//! 256 KiB local store is the canonical example, but any execution
//! unit that holds a private byte region the runtime never makes
//! globally visible can use this type.
//!
//! Critically, **local store writes do not flow through the commit
//! pipeline.** No other unit can ever observe a `LocalStore`'s
//! contents, so there is no determinism hazard from immediate
//! mutation. The "publish only via Effect" rule is about
//! *guest-visible* state; local store is not guest-visible to
//! anyone but its owning unit. The unit's `run_until_yield` may read
//! and write its own local store freely.
//!
//! What `LocalStore` is **not**:
//!
//! - It is not a cache of `GuestMemory`. There is no automatic
//!   coherence; movements between local store and global memory go
//!   through DMA effects, which the runtime translates into commit
//!   pipeline activity.
//! - It is not shared across units. Two units never reference the
//!   same `LocalStore` instance. The runtime is responsible for
//!   ensuring this.

use crate::guest::MemError;
use crate::range::ByteRange;

/// A unit-private flat byte region.
///
/// Construct with [`LocalStore::new`] sized in bytes. Reads and writes
/// are bounds-checked against the configured size; out-of-range
/// operations return `Err`/`None` rather than panicking, since the
/// owning unit may compute its own addresses and a local-store fault
/// is a guest-induced condition rather than a runtime invariant
/// violation.
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

    /// Read the bytes covered by `range`.
    ///
    /// Returns `None` if the range extends past the end of local store.
    /// A zero-length range whose start is in bounds (or exactly at
    /// `size()`) returns an empty slice -- consistent with
    /// [`crate::GuestMemory::read`].
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

    /// Write `bytes` to `range`.
    ///
    /// Unlike [`crate::GuestMemory::apply_commit`], this is the unit's
    /// own write -- it is not gated by the commit pipeline because
    /// nothing outside this unit can observe the result. The owning
    /// unit may call this freely from inside `run_until_yield`.
    ///
    /// Returns `Err(MemError::LengthMismatch)` if `bytes.len() as u64
    /// != range.length()`, and `Err(MemError::OutOfRange)` if the
    /// range extends past the end of local store.
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
        // Surrounding bytes still zero.
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
