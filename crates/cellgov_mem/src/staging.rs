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

const INLINE_BYTES: usize = 16;

/// Bytes held by a staged write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StagedBytes {
    /// Small payload stored with the staging entry.
    Inline {
        /// Fixed storage for a small payload.
        buf: [u8; INLINE_BYTES],
        /// Number of initialized bytes in `buf`.
        len: u8,
    },
    /// Large payload that requires heap storage.
    Heap(Vec<u8>),
}

impl StagedBytes {
    /// Copy `bytes`, using inline storage when possible.
    #[inline]
    pub fn from_slice(bytes: &[u8]) -> Self {
        if bytes.len() <= INLINE_BYTES {
            let mut buf = [0; INLINE_BYTES];
            buf[..bytes.len()].copy_from_slice(bytes);
            Self::Inline {
                buf,
                len: bytes.len() as u8,
            }
        } else {
            Self::Heap(bytes.to_vec())
        }
    }

    /// Borrow the staged payload.
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        match self {
            Self::Inline { buf, len } => &buf[..*len as usize],
            Self::Heap(bytes) => bytes,
        }
    }

    /// Number of bytes in the payload.
    #[inline]
    pub fn len(&self) -> usize {
        self.as_slice().len()
    }

    /// Whether the payload has no bytes.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A single staged write awaiting commit.
///
/// `bytes.len() as u64 == range.length()` is the caller's invariant; drain
/// rechecks it and rejects the whole batch on mismatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedWrite {
    /// Target byte range in committed memory.
    pub range: ByteRange,
    /// Bytes to deposit into `range` at commit time.
    pub bytes: StagedBytes,
}

impl StagedWrite {
    /// Construct a staged write, storing small payloads inline.
    #[inline]
    pub fn new(range: ByteRange, bytes: &[u8]) -> Self {
        Self {
            range,
            bytes: StagedBytes::from_slice(bytes),
        }
    }
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
        self.drain_into_observed(target, |_, _| {})
    }

    /// Drain like [`Self::drain_into`] and hand each applied write to `observe`.
    ///
    /// On a refused batch, the drain calls `observe` for no write. The
    /// caller has the reservation clear-sweep obligation that
    /// [`Self::drain_into`] names.
    ///
    /// # Errors
    ///
    /// Returns the same [`MemError`] as [`Self::drain_into`].
    pub fn drain_into_observed(
        &mut self,
        target: &mut GuestMemory,
        mut observe: impl FnMut(ByteRange, &[u8]),
    ) -> Result<usize, MemError> {
        self.validate_pending(target)?;
        let count = self.pending.len();
        for w in self.pending.drain(..) {
            target.apply_commit(w.range, w.bytes.as_slice()).expect(
                "validate_pending called validate_write; apply_commit calls the same predicate, \
                 so this Err path is structurally unreachable",
            );
            observe(w.range, w.bytes.as_slice());
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
