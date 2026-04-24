//! Pending shared-write batch awaiting commit.
//!
//! The commit pipeline pre-sorts writes by ordering key before staging;
//! [`StagingMemory`] applies them in stage order. Overlap policy is
//! "last-staged-wins" by plain in-order overwrite.
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

    /// Discard every staged write without applying any.
    #[inline]
    pub fn clear(&mut self) {
        self.pending.clear();
    }

    /// Apply every staged write to `target` in stage order, draining the buffer.
    ///
    /// Atomic: a pre-pass validates bounds and length for every entry; any
    /// failure leaves both the staging buffer and `target` untouched.
    ///
    /// # Errors
    ///
    /// Returns any [`MemError`] that [`GuestMemory::apply_commit`] would
    /// produce on the first offending write.
    pub fn drain_into(&mut self, target: &mut GuestMemory) -> Result<usize, MemError> {
        for w in &self.pending {
            if w.bytes.len() as u64 != w.range.length() {
                return Err(MemError::LengthMismatch);
            }
            let start = w.range.start().raw();
            let length = w.range.length();
            let _end = start.checked_add(length).ok_or(MemError::OutOfRange)?;
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
        s.stage(staged(0, &[1, 1, 1, 1]));
        s.stage(StagedWrite {
            range: range(4, 4),
            bytes: vec![9, 9],
        });
        let err = s.drain_into(&mut mem).unwrap_err();
        assert_eq!(err, MemError::LengthMismatch);
        assert_eq!(mem.read(range(0, 8)).unwrap(), &[0; 8]);
        assert_eq!(s.len(), 2);
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
        let b = a.clone();
        a.clear();
        assert!(a.is_empty());
        assert_eq!(b.len(), 1);
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
    }
}
