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

    /// View all pending writes in stage order. Test-only: production
    /// callers do not inspect the buffer; they stage, drain, or clear.
    #[cfg(test)]
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

    /// Discard every staged write without applying any.
    #[inline]
    pub fn clear(&mut self) {
        self.pending.clear();
    }

    /// Validate every pending write against `target`'s region map without
    /// mutating either. Returns the same [`MemError`] set
    /// [`GuestMemory::apply_commit`] produces, in stage order; this
    /// structural agreement is what lets [`Self::drain_into`] apply with
    /// an infallible `expect` after a successful validation pass.
    ///
    /// `&GuestMemory` here is intentional: the apply pass takes
    /// `&mut GuestMemory`, but the borrow checker prevents any other
    /// mutation between validation and apply, so the validation result
    /// stays accurate.
    fn validate_pending(&self, target: &GuestMemory) -> Result<(), MemError> {
        for w in &self.pending {
            if w.bytes.len() as u64 != w.range.length() {
                return Err(MemError::LengthMismatch);
            }
            let start = w.range.start().raw();
            let length = w.range.length();
            // `ByteRange::new` validated `start + length` fits u64.
            match target.containing_region(start, length) {
                None => return Err(MemError::Unmapped(target.fault_context(start))),
                Some(r) if r.access() != crate::RegionAccess::ReadWrite => {
                    return Err(MemError::ReservedWrite {
                        addr: start,
                        region: r.label(),
                    });
                }
                Some(_) => {}
            }
        }
        Ok(())
    }

    /// Apply every staged write to `target` in stage order, draining the buffer.
    ///
    /// Atomic: validation runs once over the whole batch via
    /// [`Self::validate_pending`]; on any failure both the staging buffer
    /// and `target` are untouched.
    ///
    /// # Reservation clear-sweep contract
    ///
    /// Successful drain commits bytes to main memory but does **not**
    /// fire the reservation clear-sweep. Every caller must clear any
    /// `ReservationTable` entries that overlap the committed ranges
    /// (per CBEA sec. 11.4 lock-line reservation lost semantics) or
    /// cross-unit `LL/SC` will silently lose the snoop. The commit
    /// pipeline at `cellgov_core::commit` does this per-effect-type
    /// because some effects (e.g. `ConditionalStore`) need the
    /// emitter's own reservation removed in addition to the covering
    /// sweep -- nuance a flat "ranges committed" report would lose.
    /// New callers replicating the drain pattern must re-implement
    /// the per-effect sweep at their call site.
    ///
    /// # Errors
    ///
    /// Returns any [`MemError`] that [`GuestMemory::apply_commit`] would
    /// produce on the first offending write.
    pub fn drain_into(&mut self, target: &mut GuestMemory) -> Result<usize, MemError> {
        self.validate_pending(target)?;
        let count = self.pending.len();
        for w in self.pending.drain(..) {
            // `validate_pending` ruled out LengthMismatch, Unmapped, and
            // ReservedWrite for every entry above. The `&mut GuestMemory`
            // exclusive borrow guarantees no intervening mutation could
            // invalidate that result. Any panic here means the structural
            // agreement between `validate_pending` and `apply_commit` has
            // drifted and the contract on this module is broken.
            target.apply_commit(w.range, &w.bytes).expect(
                "drain_into invariant: validate_pending must agree with apply_commit's error set",
            );
        }
        Ok(count)
    }
}

impl Drop for StagingMemory {
    fn drop(&mut self) {
        // Defense-in-depth: a non-empty drop means the commit pipeline
        // forgot to drain on success or clear on fault. Catching it
        // here turns a silent intent leak into a debug-build panic at
        // the seam where the contract operates. Skip during an active
        // unwind so a double-panic does not abort.
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
        s.clear();
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
    fn drain_into_length_mismatch_rejects_whole_batch_with_neighbors_intact() {
        let mut mem = GuestMemory::new(12);
        let mut s = StagingMemory::new();
        // (good, bad, good): all three must remain pending and memory
        // untouched. Asserts that the validator stops on the first
        // offender without partially applying anything before it.
        s.stage(staged(0, &[1, 1, 1, 1]));
        s.stage(StagedWrite {
            range: range(4, 4),
            bytes: vec![9, 9],
        });
        s.stage(staged(8, &[2, 2, 2, 2]));
        let err = s.drain_into(&mut mem).unwrap_err();
        assert_eq!(err, MemError::LengthMismatch);
        assert_eq!(mem.read(range(0, 12)).unwrap(), &[0; 12]);
        assert_eq!(s.len(), 3);
        s.clear();
    }

    #[test]
    fn drain_into_out_of_range_rejects_whole_batch() {
        let mut mem = GuestMemory::new(8);
        let mut s = StagingMemory::new();
        s.stage(staged(0, &[1, 1, 1, 1]));
        s.stage(staged(6, &[2, 2, 2, 2]));
        let err = s.drain_into(&mut mem).unwrap_err();
        assert!(matches!(err, MemError::Unmapped(_)));
        assert_eq!(mem.read(range(0, 8)).unwrap(), &[0; 8]);
        assert_eq!(s.len(), 2);
        s.clear();
    }

    #[test]
    fn drain_then_drain_again_is_noop() {
        let mut mem = GuestMemory::new(4);
        let mut s = StagingMemory::new();
        s.stage(staged(0, &[5, 5, 5, 5]));
        s.drain_into(&mut mem).unwrap();
        let n = s.drain_into(&mut mem).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn clone_is_independent() {
        let mut a = StagingMemory::new();
        a.stage(staged(0, &[1, 2, 3]));
        let mut b = a.clone();
        a.clear();
        assert!(a.is_empty());
        assert_eq!(b.len(), 1);
        b.clear();
    }

    #[test]
    fn drain_into_reserved_region_returns_reserved_write() {
        use crate::{PageSize, Region, RegionAccess};
        let mut mem = GuestMemory::from_regions(vec![
            Region::new(0, 256, "main", PageSize::Page64K),
            Region::with_access(
                0xC000_0000,
                256,
                "rsx",
                PageSize::Page64K,
                RegionAccess::ReservedZeroReadable,
            ),
        ])
        .unwrap();
        let mut s = StagingMemory::new();
        s.stage(staged(0xC000_0000, &[1, 2, 3, 4]));
        let err = s.drain_into(&mut mem).unwrap_err();
        assert!(
            matches!(err, MemError::ReservedWrite { region: "rsx", .. }),
            "expected ReservedWrite, got {err:?}"
        );
        s.clear();
    }

    #[test]
    fn drain_into_reserved_strict_region_returns_reserved_write() {
        use crate::{PageSize, Region, RegionAccess};
        let mut mem = GuestMemory::from_regions(vec![
            Region::new(0, 256, "main", PageSize::Page64K),
            Region::with_access(
                0xE000_0000,
                256,
                "spu_reserved",
                PageSize::Page64K,
                RegionAccess::ReservedStrict,
            ),
        ])
        .unwrap();
        let mut s = StagingMemory::new();
        s.stage(staged(0xE000_0000, &[1, 2, 3, 4]));
        let err = s.drain_into(&mut mem).unwrap_err();
        assert!(
            matches!(
                err,
                MemError::ReservedWrite {
                    region: "spu_reserved",
                    ..
                }
            ),
            "expected ReservedWrite, got {err:?}"
        );
        s.clear();
    }

    #[test]
    fn drain_into_cross_region_span_rejects_whole_batch() {
        use crate::{PageSize, Region};
        let mut mem = GuestMemory::from_regions(vec![
            Region::new(0, 0x100, "main", PageSize::Page64K),
            Region::new(0x100, 0x100, "tail", PageSize::Page64K),
        ])
        .unwrap();
        let mut s = StagingMemory::new();
        // [0xFC, 0x104): straddles the main/tail boundary at 0x100.
        s.stage(staged(0xFC, &[1, 2, 3, 4, 5, 6, 7, 8]));
        let err = s.drain_into(&mut mem).unwrap_err();
        assert!(matches!(err, MemError::Unmapped(_)));
        assert_eq!(s.len(), 1, "rejected batch must remain intact for retry");
        s.clear();
    }

    #[test]
    fn drain_into_zero_length_write_to_rw_region_is_noop() {
        let mut mem = GuestMemory::new(8);
        mem.apply_commit(range(0, 8), &[1, 2, 3, 4, 5, 6, 7, 8])
            .unwrap();
        let mut s = StagingMemory::new();
        s.stage(staged(4, &[]));
        let n = s.drain_into(&mut mem).unwrap();
        assert_eq!(n, 1);
        assert!(s.is_empty());
        assert_eq!(mem.read(range(0, 8)).unwrap(), &[1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn drain_into_zero_length_write_to_reserved_region_faults() {
        use crate::{PageSize, Region, RegionAccess};
        let mut mem = GuestMemory::from_regions(vec![Region::with_access(
            0xC000_0000,
            256,
            "rsx",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        )])
        .unwrap();
        let mut s = StagingMemory::new();
        s.stage(staged(0xC000_0000, &[]));
        let err = s.drain_into(&mut mem).unwrap_err();
        assert!(
            matches!(err, MemError::ReservedWrite { region: "rsx", .. }),
            "expected ReservedWrite, got {err:?}"
        );
        s.clear();
    }

    #[test]
    fn drop_with_pending_writes_panics_in_debug() {
        // Sanity check on the Drop debug_assert: a leak of staged writes
        // panics in debug builds. Release builds skip the assert.
        if cfg!(debug_assertions) {
            let result = std::panic::catch_unwind(|| {
                let mut s = StagingMemory::new();
                s.stage(staged(0, &[1]));
                // s drops here with one pending write.
            });
            assert!(result.is_err(), "expected debug-build panic on leak");
        }
    }
}
