//! `GuestMemory` -- committed globally visible memory.
//!
//! `GuestMemory` is the runtime's source of truth for what every unit
//! sees when it reads from globally visible memory. It is read-only from
//! the perspective of execution units; the only mutation entry point is
//! [`GuestMemory::apply_commit`], which the commit pipeline in
//! `cellgov_core` invokes after validating a batch of `SharedWriteIntent`
//! effects. Execution units never call `apply_commit` directly; no
//! execution unit may publish guest-visible state directly.
//!
//! Currently `GuestMemory` is backed by a flat `Vec<u8>` of fixed size.
//! This is deliberately simple; future additions such as reservation
//! granules, barriers, page permissions, and MMIO regions should slot
//! in without changing the top-level interfaces. The two operations
//! exposed here ([`read`](GuestMemory::read) and
//! [`apply_commit`](GuestMemory::apply_commit)) are deliberately minimal
//! so that any of those future layers can wrap the same shape.

use crate::range::ByteRange;

/// Committed globally visible guest memory.
///
/// Construct with [`GuestMemory::new`] sized in bytes. Reads are checked
/// against the configured size; out-of-range reads return `None` rather
/// than panicking, since reading past the end of guest memory is a
/// guest-induced fault, not a runtime invariant violation.
#[derive(Debug, Clone)]
pub struct GuestMemory {
    bytes: Vec<u8>,
}

/// Why a `GuestMemory` operation failed.
///
/// `MemError` is local to `cellgov_mem`; there is no universal `Error`
/// enum spanning the workspace. Boundary crates that consume
/// these errors -- the commit pipeline in `cellgov_core` and the
/// validation layer in `cellgov_effects` -- convert into their own
/// error types at the seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemError {
    /// The requested range extends past the end of guest memory.
    OutOfRange,
    /// The supplied byte buffer's length does not match the range's
    /// length. Only relevant for [`GuestMemory::apply_commit`].
    LengthMismatch,
}

impl GuestMemory {
    /// Construct a fresh `GuestMemory` of `size` bytes, zero-initialized.
    ///
    /// Takes a `usize` rather than a `u64` because the backing is a
    /// `Vec<u8>` and `Vec` is `usize`-indexed. The `u64` address
    /// space exposed through `GuestAddr` is wider than the host can
    /// represent contiguously; future sparse-page or MMIO-aware backings
    /// will lift this restriction without changing the read/commit API.
    #[inline]
    pub fn new(size: usize) -> Self {
        Self {
            bytes: vec![0u8; size],
        }
    }

    /// Total committed memory size in bytes.
    #[inline]
    pub fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    /// Read the entire committed memory backing as a byte slice.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Read the bytes covered by `range`.
    ///
    /// Returns `None` if the range extends past the end of memory or
    /// if the range's start is itself out of bounds. A zero-length
    /// range whose start is in bounds (or exactly at `size()`) returns
    /// an empty slice; the runtime trace still records the read.
    pub fn read(&self, range: ByteRange) -> Option<&[u8]> {
        let start = range.start().raw();
        let end = start.checked_add(range.length())?;
        let size = self.size();
        if end > size {
            return None;
        }
        // Safe: end <= size <= bytes.len() (the latter by construction
        // of `bytes` from a `usize`-sized `vec!`).
        let start_usize = usize::try_from(start).ok()?;
        let end_usize = usize::try_from(end).ok()?;
        Some(&self.bytes[start_usize..end_usize])
    }

    /// Apply a committed write to `range` from `bytes`.
    ///
    /// **This is the commit pipeline's entry point and should not be
    /// called from execution units.** No execution unit may publish
    /// guest-visible state directly; this method exists only so that
    /// `cellgov_core`'s commit applier has a typed seam to call.
    /// Currently nothing enforces this at the language level -- the
    /// rule is architectural -- but the seam is narrow enough that
    /// violations are visible at code review.
    ///
    /// Returns `Err(MemError::LengthMismatch)` if `bytes.len() as u64
    /// != range.length()`, and `Err(MemError::OutOfRange)` if the
    /// range extends past the end of memory.
    pub fn apply_commit(&mut self, range: ByteRange, bytes: &[u8]) -> Result<(), MemError> {
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

    /// 64-bit content hash of every committed byte, in address order.
    ///
    /// FNV-1a, no external deps, no host-specific seeding. Checkpoint
    /// hashes must be deterministic across hosts and runs; FNV-1a
    /// satisfies that and is small enough to inline. Replay tooling
    /// compares pairs of these values to assert that two runs reached
    /// bit-identical committed-memory state. Currently hashes the
    /// entire backing buffer; future sparse-page or MMIO-aware
    /// backings will fold their state into the same `u64` output without
    /// changing this method's signature.
    pub fn content_hash(&self) -> u64 {
        crate::hash::fnv1a(&self.bytes)
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
        let mem = GuestMemory::new(16);
        assert_eq!(mem.size(), 16);
        let bytes = mem.read(range(0, 16)).unwrap();
        assert_eq!(bytes, &[0u8; 16]);
    }

    #[test]
    fn read_in_range() {
        let mut mem = GuestMemory::new(16);
        mem.apply_commit(range(4, 4), &[1, 2, 3, 4]).unwrap();
        assert_eq!(mem.read(range(4, 4)).unwrap(), &[1, 2, 3, 4]);
    }

    #[test]
    fn read_zero_length_at_in_bounds_start() {
        let mem = GuestMemory::new(16);
        let s = mem.read(range(8, 0)).unwrap();
        assert!(s.is_empty());
    }

    #[test]
    fn read_zero_length_at_size_boundary() {
        let mem = GuestMemory::new(16);
        let s = mem.read(range(16, 0)).unwrap();
        assert!(s.is_empty());
    }

    #[test]
    fn read_past_end_is_none() {
        let mem = GuestMemory::new(16);
        assert_eq!(mem.read(range(15, 2)), None);
    }

    #[test]
    fn read_starting_past_end_is_none() {
        let mem = GuestMemory::new(16);
        assert_eq!(mem.read(range(17, 1)), None);
    }

    #[test]
    fn commit_writes_visible_on_read() {
        let mut mem = GuestMemory::new(8);
        mem.apply_commit(range(0, 4), &[0xde, 0xad, 0xbe, 0xef])
            .unwrap();
        assert_eq!(mem.read(range(0, 4)).unwrap(), &[0xde, 0xad, 0xbe, 0xef]);
        // Untouched tail still zero.
        assert_eq!(mem.read(range(4, 4)).unwrap(), &[0, 0, 0, 0]);
    }

    #[test]
    fn commit_length_mismatch_rejected() {
        let mut mem = GuestMemory::new(16);
        let err = mem.apply_commit(range(0, 4), &[1, 2, 3]).unwrap_err();
        assert_eq!(err, MemError::LengthMismatch);
        // Memory left untouched.
        assert_eq!(mem.read(range(0, 4)).unwrap(), &[0, 0, 0, 0]);
    }

    #[test]
    fn commit_out_of_range_rejected() {
        let mut mem = GuestMemory::new(8);
        let err = mem.apply_commit(range(6, 4), &[1, 2, 3, 4]).unwrap_err();
        assert_eq!(err, MemError::OutOfRange);
        // Memory left untouched.
        assert_eq!(mem.read(range(0, 8)).unwrap(), &[0; 8]);
    }

    #[test]
    fn commit_zero_length_is_noop() {
        let mut mem = GuestMemory::new(8);
        mem.apply_commit(range(4, 0), &[]).unwrap();
        assert_eq!(mem.read(range(0, 8)).unwrap(), &[0; 8]);
    }

    #[test]
    fn overlapping_commits_apply_in_call_order() {
        // The commit pipeline guarantees deterministic ordering at a
        // higher level; this test just confirms `apply_commit` itself
        // does not silently buffer or reorder.
        let mut mem = GuestMemory::new(8);
        mem.apply_commit(range(0, 4), &[1, 1, 1, 1]).unwrap();
        mem.apply_commit(range(2, 4), &[2, 2, 2, 2]).unwrap();
        assert_eq!(mem.read(range(0, 8)).unwrap(), &[1, 1, 2, 2, 2, 2, 0, 0]);
    }

    #[test]
    fn content_hash_of_zero_initialized_is_stable() {
        let a = GuestMemory::new(16);
        let b = GuestMemory::new(16);
        assert_eq!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn content_hash_changes_on_commit() {
        let mut mem = GuestMemory::new(8);
        let before = mem.content_hash();
        mem.apply_commit(range(0, 4), &[1, 2, 3, 4]).unwrap();
        let after = mem.content_hash();
        assert_ne!(before, after);
    }

    #[test]
    fn content_hash_is_size_sensitive() {
        // Two zero-initialized buffers of different sizes must hash
        // differently, otherwise replay would mistake a 16-byte zero
        // memory for an 8-byte zero memory.
        let a = GuestMemory::new(8);
        let b = GuestMemory::new(16);
        assert_ne!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn content_hash_is_position_sensitive() {
        // Same bytes at different addresses must produce different
        // hashes; FNV-1a's order dependence guarantees this.
        let mut a = GuestMemory::new(8);
        let mut b = GuestMemory::new(8);
        a.apply_commit(range(0, 1), &[0xff]).unwrap();
        b.apply_commit(range(4, 1), &[0xff]).unwrap();
        assert_ne!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn content_hash_round_trips_after_revert() {
        // Hashing is a pure function of bytes: writing X then writing
        // back the original bytes restores the original hash.
        let mut mem = GuestMemory::new(4);
        let h0 = mem.content_hash();
        mem.apply_commit(range(0, 4), &[1, 2, 3, 4]).unwrap();
        assert_ne!(mem.content_hash(), h0);
        mem.apply_commit(range(0, 4), &[0, 0, 0, 0]).unwrap();
        assert_eq!(mem.content_hash(), h0);
    }

    #[test]
    fn clone_is_independent() {
        let mut a = GuestMemory::new(4);
        a.apply_commit(range(0, 4), &[9, 9, 9, 9]).unwrap();
        let b = a.clone();
        a.apply_commit(range(0, 4), &[0, 0, 0, 0]).unwrap();
        assert_eq!(a.read(range(0, 4)).unwrap(), &[0, 0, 0, 0]);
        assert_eq!(b.read(range(0, 4)).unwrap(), &[9, 9, 9, 9]);
    }
}
