//! Pending shared-write batch awaiting commit.
//!
//! [`StagingMemory`] preserves the order in which [`StagingMemory::stage`]
//! is called and applies writes in that order at [`StagingMemory::drain_into`]
//! time. It does not sort, deduplicate, or otherwise interpret the writes;
//! emission-order determinism (the "ordering key" the rest of the runtime
//! relies on for `sync_state_hash`) is the caller's invariant. Overlap
//! policy is "last-staged-wins" by plain in-order overwrite.
//!
//! Zero-length staged writes are accepted: their region is resolved like
//! any other write, so a zero-length write to a `ReadWrite` region is a
//! no-op and a zero-length write to a reserved region still faults with
//! [`MemError::ReservedWrite`]. Region resolution at an exact boundary
//! address picks the region whose base equals that address (half-open
//! `[base, end)` intuition).
//!
//! A faulting step commits nothing: the runtime calls [`StagingMemory::clear`]
//! before draining when the originating step yielded a fault.

use crate::guest::{GuestMemory, MemError};
use crate::range::ByteRange;

/// A single staged write awaiting commit.
///
/// `bytes.len() as u64 == range.length()` is the caller's invariant; drain
/// rechecks it and rejects the whole batch on mismatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedWrite {
    /// Target byte range in committed memory.
    pub range: ByteRange,
    /// Bytes to deposit into `range` at commit time.
    pub bytes: Vec<u8>,
}

/// A buffer of staged writes pending commit.
#[derive(Debug, Default, Clone)]
pub struct StagingMemory {
    pending: Vec<StagedWrite>,
}

impl StagingMemory {
    /// Construct an empty staging buffer.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a staged write. Stage order equals application order.
    #[inline]
    pub fn stage(&mut self, write: StagedWrite) {
        self.pending.push(write);
    }

    /// Number of writes currently buffered.
    #[inline]
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether the buffer is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Discard every staged write without applying any.
    #[inline]
    pub fn clear(&mut self) {
        self.pending.clear();
    }

    /// Validate every pending write against `target` via
    /// [`GuestMemory::validate_write`] -- the predicate `apply_commit`
    /// itself uses -- and return on the first failure. Mutates neither
    /// the staging buffer nor `target`.
    fn validate_pending(&self, target: &GuestMemory) -> Result<(), MemError> {
        for w in &self.pending {
            target.validate_write(w.range, w.bytes.len())?;
        }
        Ok(())
    }

    /// Apply every staged write to `target` in stage order, draining the buffer.
    ///
    /// Atomic: validation runs once over the whole batch via
    /// `Self::validate_pending`; on any failure both the staging buffer
    /// and `target` are untouched.
    ///
    /// # Reservation clear-sweep contract
    ///
    /// Successful drain commits bytes to main memory but does **not**
    /// fire the reservation clear-sweep. Every caller must clear any
    /// `ReservationTable` entries that overlap the committed ranges
    /// (per [PPC-Book2 p:23 s:3.3.2] lock-line reservation semantics)
    /// or cross-unit `LL/SC` will silently lose the snoop.
    ///
    /// # Errors
    ///
    /// Returns any [`MemError`] that [`GuestMemory::apply_commit`] would
    /// produce on the first offending write.
    pub fn drain_into(&mut self, target: &mut GuestMemory) -> Result<usize, MemError> {
        self.validate_pending(target)?;
        let count = self.pending.len();
        for w in self.pending.drain(..) {
            target.apply_commit(w.range, &w.bytes).expect(
                "validate_pending called validate_write; apply_commit calls the same predicate, \
                 so this Err path is structurally unreachable",
            );
        }
        Ok(count)
    }
}

impl Drop for StagingMemory {
    fn drop(&mut self) {
        // Skip during an active unwind so a double-panic does not abort.
        if std::thread::panicking() {
            return;
        }
        debug_assert!(
            self.pending.is_empty(),
            "StagingMemory dropped with {} pending writes -- the commit pipeline must call \
             drain_into() or clear() before the buffer is released",
            self.pending.len()
        );
    }
}

#[cfg(test)]
#[path = "tests/staging_tests.rs"]
mod tests;
