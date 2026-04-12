//! `StagingMemory` -- pending shared visibility changes awaiting commit.
//!
//! `StagingMemory` is the buffer the commit pipeline fills with the byte
//! payloads of `SharedWriteIntent` effects after validation but before
//! application. It holds them until the pipeline drains the batch into
//! [`crate::guest::GuestMemory`] at an epoch boundary.
//!
//! What it deliberately is **not**:
//!
//! - It does not know about `cellgov_effects::Effect` or `OrderingKey`
//!   types. The commit pipeline pre-sorts writes by the global ordering
//!   key (timestamp, priority class, source unit id, sequence number)
//!   before staging them; `StagingMemory` applies them in the order they
//!   were staged. Keeping the ordering policy out of this crate avoids
//!   the layering pressure that would push `cellgov_mem` to depend on
//!   `cellgov_event` for `OrderingKey`.
//! - It does not detect or resolve overlapping writes. Overlap policy is
//!   "last writer wins by ordering key", which the commit pipeline
//!   already encoded by sorting before staging; the same byte ranges
//!   simply overwrite each other when drained in order.
//! - It does not validate effects beyond the bounds checks that
//!   [`StagingMemory::drain_into`] performs in its pre-pass. Earlier
//!   validation lives in `cellgov_effects`.
//!
//! A faulting step commits nothing. The runtime implements this by
//! calling [`StagingMemory::clear`] before draining when the
//! originating step yielded with `YieldReason::Fault`. This type just
//! exposes the operations; the policy lives in the runtime.

use crate::guest::{GuestMemory, MemError};
use crate::range::ByteRange;

/// A single staged write awaiting commit.
///
/// Carries the target range and the bytes to deposit. The byte buffer's
/// length must match `range.length()`; this is checked at drain time
/// rather than at construction so that the type can be built up cheaply
/// and rejected wholesale if the batch is malformed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedWrite {
    /// Target byte range in committed memory.
    pub range: ByteRange,
    /// Bytes to deposit into `range` at commit time.
    pub bytes: Vec<u8>,
}

/// A buffer of staged writes pending commit.
///
/// `StagingMemory` is owned by the commit pipeline. Units never see it
/// directly; they emit `SharedWriteIntent` effects which the pipeline
/// translates into [`StagedWrite`] entries and stages here. At the
/// epoch boundary the pipeline drains the staging buffer into
/// [`GuestMemory`].
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

    /// Append a staged write to the buffer. Order is preserved -- the
    /// commit pipeline relies on the order it stages writes matching
    /// the order they will be applied.
    #[inline]
    pub fn stage(&mut self, write: StagedWrite) {
        self.pending.push(write);
    }

    /// View all pending writes in stage order.
    #[inline]
    pub fn pending(&self) -> &[StagedWrite] {
        &self.pending
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

    /// Discard every staged write without applying any. Used by the
    /// commit pipeline when a step yields with a fault: the entire
    /// step commits nothing, including effects that preceded the fault
    /// in emission order.
    #[inline]
    pub fn clear(&mut self) {
        self.pending.clear();
    }

    /// Apply every staged write to `target` in stage order, draining
    /// the buffer.
    ///
    /// Two-pass to keep the contract atomic: the first pass validates
    /// every staged write against `target`'s bounds and length rules;
    /// the second pass applies them. Either every write becomes visible
    /// or none do, preserving atomic-batch semantics at the granularity
    /// this type owns. Validation failure leaves the
    /// staging buffer untouched and `target` untouched.
    ///
    /// Returns the number of writes applied on success.
    pub fn drain_into(&mut self, target: &mut GuestMemory) -> Result<usize, MemError> {
        // Pre-validate the whole batch.
        let size = target.size();
        for w in &self.pending {
            if w.bytes.len() as u64 != w.range.length() {
                return Err(MemError::LengthMismatch);
            }
            let end = w
                .range
                .start()
                .raw()
                .checked_add(w.range.length())
                .ok_or(MemError::OutOfRange)?;
            if end > size {
                return Err(MemError::OutOfRange);
            }
        }
        // All writes are valid; apply them in order. apply_commit
        // re-checks but cannot fail given the pre-pass.
        let count = self.pending.len();
        for w in self.pending.drain(..) {
            target
                .apply_commit(w.range, &w.bytes)
                .expect("pre-validated above");
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::addr::GuestAddr;

    fn range(start: u64, length: u64) -> ByteRange {
        ByteRange::new(GuestAddr::new(start), length).unwrap()
    }

    fn staged(start: u64, bytes: &[u8]) -> StagedWrite {
        StagedWrite {
            range: range(start, bytes.len() as u64),
            bytes: bytes.to_vec(),
        }
    }

    #[test]
    fn new_is_empty() {
        let s = StagingMemory::new();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
        assert!(s.pending().is_empty());
    }

    #[test]
    fn stage_preserves_order() {
        let mut s = StagingMemory::new();
        s.stage(staged(0, &[1, 1]));
        s.stage(staged(2, &[2, 2]));
        s.stage(staged(4, &[3, 3]));
        assert_eq!(s.len(), 3);
        let starts: Vec<u64> = s.pending().iter().map(|w| w.range.start().raw()).collect();
        assert_eq!(starts, vec![0, 2, 4]);
    }

    #[test]
    fn clear_discards_everything() {
        let mut s = StagingMemory::new();
        s.stage(staged(0, &[1, 2, 3, 4]));
        s.stage(staged(8, &[5, 6, 7, 8]));
        s.clear();
        assert!(s.is_empty());
    }

    #[test]
    fn drain_into_writes_in_stage_order() {
        let mut mem = GuestMemory::new(16);
        let mut s = StagingMemory::new();
        s.stage(staged(0, &[1, 1, 1, 1]));
        s.stage(staged(8, &[2, 2, 2, 2]));
        let n = s.drain_into(&mut mem).unwrap();
        assert_eq!(n, 2);
        assert!(s.is_empty());
        assert_eq!(
            mem.read(range(0, 16)).unwrap(),
            &[1, 1, 1, 1, 0, 0, 0, 0, 2, 2, 2, 2, 0, 0, 0, 0]
        );
    }

    #[test]
    fn drain_into_overlapping_last_writer_wins_in_stage_order() {
        // The commit pipeline is responsible for sorting by ordering
        // key before staging. From StagingMemory's standpoint the
        // contract is "later stage entries overwrite earlier ones on
        // overlap" -- which is exactly what plain in-order application
        // produces.
        let mut mem = GuestMemory::new(8);
        let mut s = StagingMemory::new();
        s.stage(staged(0, &[1, 1, 1, 1]));
        s.stage(staged(2, &[2, 2, 2, 2]));
        s.drain_into(&mut mem).unwrap();
        assert_eq!(mem.read(range(0, 8)).unwrap(), &[1, 1, 2, 2, 2, 2, 0, 0]);
    }

    #[test]
    fn drain_into_empty_is_ok_and_noop() {
        let mut mem = GuestMemory::new(4);
        mem.apply_commit(range(0, 4), &[7, 7, 7, 7]).unwrap();
        let mut s = StagingMemory::new();
        let n = s.drain_into(&mut mem).unwrap();
        assert_eq!(n, 0);
        assert_eq!(mem.read(range(0, 4)).unwrap(), &[7, 7, 7, 7]);
    }

    #[test]
    fn drain_into_length_mismatch_rejects_whole_batch() {
        let mut mem = GuestMemory::new(8);
        let mut s = StagingMemory::new();
        s.stage(staged(0, &[1, 1, 1, 1])); // valid
                                           // Construct a malformed write directly: range says 4 bytes, payload 2.
        s.stage(StagedWrite {
            range: range(4, 4),
            bytes: vec![9, 9],
        });
        let err = s.drain_into(&mut mem).unwrap_err();
        assert_eq!(err, MemError::LengthMismatch);
        // Memory left untouched: even the valid first write is not applied.
        assert_eq!(mem.read(range(0, 8)).unwrap(), &[0; 8]);
        // Staging buffer left intact for inspection.
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn drain_into_out_of_range_rejects_whole_batch() {
        let mut mem = GuestMemory::new(8);
        let mut s = StagingMemory::new();
        s.stage(staged(0, &[1, 1, 1, 1])); // valid
        s.stage(staged(6, &[2, 2, 2, 2])); // overflows: end = 10 > 8
        let err = s.drain_into(&mut mem).unwrap_err();
        assert_eq!(err, MemError::OutOfRange);
        // Memory left untouched.
        assert_eq!(mem.read(range(0, 8)).unwrap(), &[0; 8]);
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn drain_then_drain_again_is_noop() {
        let mut mem = GuestMemory::new(4);
        let mut s = StagingMemory::new();
        s.stage(staged(0, &[5, 5, 5, 5]));
        s.drain_into(&mut mem).unwrap();
        // Buffer empty after first drain.
        let n = s.drain_into(&mut mem).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn clone_is_independent() {
        let mut a = StagingMemory::new();
        a.stage(staged(0, &[1, 2, 3]));
        let b = a.clone();
        a.clear();
        assert!(a.is_empty());
        assert_eq!(b.len(), 1);
    }
}
