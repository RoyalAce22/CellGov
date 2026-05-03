//! Committed globally visible memory.
//!
//! Mutation entry point is [`GuestMemory::apply_commit`], invoked by the
//! commit pipeline in `cellgov_core` after it validates a batch of
//! `SharedWriteIntent` effects. Execution units must not call it directly.
//!
//! Backing: a `Vec<Region>` sorted by base address; lookups use
//! `partition_point`. Region counts stay single-digit, so the linear scan
//! is faster than a `BTreeMap` walk.

use std::cell::Cell;

use crate::range::ByteRange;

/// Page-size class of a region. Informational; no page-granular protection
/// is enforced by the region map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PageSize {
    /// 4 KB pages.
    Page4K,
    /// 64 KB pages. Default for PS3 LV2 user memory.
    Page64K,
    /// 1 MB pages.
    Page1M,
}

/// Access mode of a region. Commit-boundary checks consult this to decide
/// whether a read or write faults and which fault variant to produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionAccess {
    /// Reads return stored bytes; writes via `apply_commit` succeed.
    ReadWrite,
    /// Reads return zero and bump `provisional_read_count`; writes fault
    /// with [`MemError::ReservedWrite`].
    ReservedZeroReadable,
    /// Reads return `None` (treated as unmapped); writes fault with
    /// [`MemError::ReservedWrite`].
    ReservedStrict,
}

/// A single contiguous guest memory region.
#[derive(Debug, Clone)]
pub struct Region {
    base: u64,
    bytes: Vec<u8>,
    label: &'static str,
    page_size: PageSize,
    access: RegionAccess,
}

impl Region {
    /// Construct a zero-filled read-write region.
    #[inline]
    pub fn new(base: u64, size: usize, label: &'static str, page_size: PageSize) -> Self {
        Self::with_access(base, size, label, page_size, RegionAccess::ReadWrite)
    }

    /// Construct a zero-filled region with an explicit access mode.
    #[inline]
    pub fn with_access(
        base: u64,
        size: usize,
        label: &'static str,
        page_size: PageSize,
        access: RegionAccess,
    ) -> Self {
        Self {
            base,
            bytes: vec![0u8; size],
            label,
            page_size,
            access,
        }
    }

    /// Access mode of the region.
    #[inline]
    pub fn access(&self) -> RegionAccess {
        self.access
    }

    /// Base guest address of the region.
    #[inline]
    pub fn base(&self) -> u64 {
        self.base
    }

    /// Size of the region in bytes.
    #[inline]
    pub fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    /// Diagnostic label for the region (e.g. `"flat"`, `"user_heap"`, `"stack"`).
    #[inline]
    pub fn label(&self) -> &'static str {
        self.label
    }

    /// Page-size class of the region.
    #[inline]
    pub fn page_size(&self) -> PageSize {
        self.page_size
    }

    /// Byte slice covering the region's full backing store.
    #[inline]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Exclusive end address, saturating at `u64::MAX`.
    #[inline]
    fn end(&self) -> u64 {
        self.base.saturating_add(self.size())
    }

    /// Whether `[addr, addr + length)` is entirely within this region.
    #[inline]
    fn contains(&self, addr: u64, length: u64) -> bool {
        match addr.checked_add(length) {
            Some(end) => addr >= self.base && end <= self.end(),
            None => false,
        }
    }
}

/// Committed globally visible guest memory.
///
/// Out-of-region reads via [`GuestMemory::read`] return `None` rather than
/// panicking: a boundary miss is a guest-induced fault, not a runtime
/// invariant violation.
#[derive(Debug, Clone)]
pub struct GuestMemory {
    regions: Vec<Region>,
    /// Cached FNV-1a digest; `None` iff a successful commit has happened
    /// since the last computation. Errors leave `cached_hash` untouched.
    /// `Cell` lets [`GuestMemory::content_hash`] take `&self`.
    cached_hash: Cell<Option<u64>>,
    /// Monotonic count of reads that hit a [`RegionAccess::ReservedZeroReadable`]
    /// region. Diagnostics surface silent zero-reads via this counter.
    /// Inherited by `clone()`; reset only by constructing a new `GuestMemory`.
    provisional_read_count: Cell<u64>,
}

/// Diagnostic context for an out-of-region access.
///
/// Carried by [`MemError::Unmapped`] so a fault at `0xB0000000` can name
/// "between `user_heap` and `rsx`" rather than just "out of bounds".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaultContext {
    /// Guest address that faulted.
    pub addr: u64,
    /// Label of the nearest mapped region whose end is `<= addr`, if any.
    pub nearest_below: Option<&'static str>,
    /// Label of the nearest mapped region whose base is `> addr`, if any.
    pub nearest_above: Option<&'static str>,
}

/// Why a `GuestMemory` operation failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemError {
    /// Supplied byte buffer's length differs from the range's length.
    LengthMismatch,
    /// `from_regions` received overlapping address ranges.
    OverlappingRegions,
    /// Range is not entirely contained within any single mapped region.
    Unmapped(FaultContext),
    /// Write targeted a non-`ReadWrite` region.
    ReservedWrite {
        /// Guest address of the faulting write.
        addr: u64,
        /// Label of the reserved region the write targeted.
        region: &'static str,
    },
    /// Read targeted a `ReservedStrict` region.
    ReservedStrictRead {
        /// Guest address of the faulting read.
        addr: u64,
        /// Label of the reserved region the read targeted.
        region: &'static str,
    },
}

impl GuestMemory {
    /// Construct a fresh `GuestMemory` with a single `"flat"` region at base 0.
    #[inline]
    pub fn new(size: usize) -> Self {
        Self::from_regions(vec![Region::new(0, size, "flat", PageSize::Page64K)])
            .expect("single region at base 0 is always non-overlapping")
    }

    /// Construct `GuestMemory` from a set of regions.
    ///
    /// # Errors
    ///
    /// Returns [`MemError::OverlappingRegions`] if any two regions' address
    /// ranges overlap. Empty input is allowed; every read then faults.
    pub fn from_regions(mut regions: Vec<Region>) -> Result<Self, MemError> {
        regions.sort_by_key(|r| r.base());
        for pair in regions.windows(2) {
            if pair[0].end() > pair[1].base() {
                return Err(MemError::OverlappingRegions);
            }
        }
        Ok(Self {
            regions,
            cached_hash: Cell::new(None),
            provisional_read_count: Cell::new(0),
        })
    }

    /// Reads that have hit a `ReservedZeroReadable` region. Persists across
    /// `clone()`; only construction resets the counter.
    #[inline]
    pub fn provisional_read_count(&self) -> u64 {
        self.provisional_read_count.get()
    }

    /// Sum of every region's size in bytes. Gaps between regions are not counted.
    #[inline]
    pub fn size(&self) -> u64 {
        self.regions.iter().map(|r| r.size()).sum()
    }

    /// All regions in address order.
    pub fn regions(&self) -> impl Iterator<Item = &Region> {
        self.regions.iter()
    }

    /// Byte slice of the region at base 0, or an empty slice if none exists.
    ///
    /// Auxiliary regions (stack, reserved RSX/SPU ranges) are not visible
    /// through this accessor; use [`GuestMemory::read`] or iterate
    /// [`GuestMemory::regions`].
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        match self.regions.first() {
            Some(r) if r.base() == 0 => &r.bytes,
            _ => &[],
        }
    }

    /// Resolve a read against the region map, applying the access-mode
    /// dispatch (`ReservedZeroReadable` bumps the provisional counter,
    /// `ReservedStrict` faults). The single source of truth shared by
    /// [`GuestMemory::read`] and [`GuestMemory::read_checked`] so the two
    /// cannot drift on access semantics. `ByteRange::new` validated
    /// `start + length` does not overflow `u64`, so no overflow check
    /// is needed here.
    fn resolve_read(&self, range: ByteRange) -> Result<&[u8], MemError> {
        let start = range.start().raw();
        let length = range.length();
        let region = self
            .containing_region(start, length)
            .ok_or_else(|| MemError::Unmapped(self.fault_context(start)))?;
        match region.access {
            RegionAccess::ReadWrite => {}
            RegionAccess::ReservedZeroReadable => {
                self.provisional_read_count
                    .set(self.provisional_read_count.get().saturating_add(1));
            }
            RegionAccess::ReservedStrict => {
                return Err(MemError::ReservedStrictRead {
                    addr: start,
                    region: region.label(),
                });
            }
        }
        // `region.contains(start, length)` already guaranteed offset and end
        // fit inside `region.bytes`, whose length is `usize` by construction.
        let offset = (start - region.base()) as usize;
        let end = offset + length as usize;
        Ok(&region.bytes[offset..end])
    }

    /// Read the bytes covered by `range`, or `None` if no region contains
    /// it or the target region is `ReservedStrict`.
    ///
    /// A read from a `ReservedZeroReadable` region bumps
    /// [`GuestMemory::provisional_read_count`]. For diagnostic-rich failure
    /// information, use [`GuestMemory::read_checked`].
    pub fn read(&self, range: ByteRange) -> Option<&[u8]> {
        self.resolve_read(range).ok()
    }

    /// Read the bytes covered by `range`, returning a typed error on failure.
    ///
    /// # Errors
    ///
    /// - [`MemError::Unmapped`] with a [`FaultContext`] when no region
    ///   contains the range.
    /// - [`MemError::ReservedStrictRead`] when the target region is
    ///   `ReservedStrict`.
    pub fn read_checked(&self, range: ByteRange) -> Result<&[u8], MemError> {
        self.resolve_read(range)
    }

    /// Apply a committed write to `range` from `bytes`. Invalidates the
    /// cached content hash on success; errors leave both memory and the
    /// cache untouched.
    ///
    /// This is the commit pipeline's entry point; execution units must not
    /// call it directly. The rule is architectural, not language-enforced.
    ///
    /// # Errors
    ///
    /// - [`MemError::LengthMismatch`] if `bytes.len()` differs from `range.length()`.
    /// - [`MemError::Unmapped`] if no region contains the range.
    /// - [`MemError::ReservedWrite`] if the target region is not `ReadWrite`.
    pub fn apply_commit(&mut self, range: ByteRange, bytes: &[u8]) -> Result<(), MemError> {
        if bytes.len() as u64 != range.length() {
            return Err(MemError::LengthMismatch);
        }
        let start = range.start().raw();
        let length = range.length();
        // `ByteRange::new` validated `start + length` fits u64.
        let fault = self.fault_context(start);
        let region = self
            .containing_region_mut(start, length)
            .ok_or(MemError::Unmapped(fault))?;
        if region.access != RegionAccess::ReadWrite {
            return Err(MemError::ReservedWrite {
                addr: start,
                region: region.label(),
            });
        }
        let offset = (start - region.base()) as usize;
        let end = offset + length as usize;
        region.bytes[offset..end].copy_from_slice(bytes);
        self.cached_hash.set(None);
        Ok(())
    }

    /// 64-bit FNV-1a digest of every region's bytes, hashed in address order.
    ///
    /// Cached; a subsequent [`GuestMemory::apply_commit`] invalidates the
    /// cache. The first call on a large memory is O(total bytes); cached
    /// lookups are O(1).
    pub fn content_hash(&self) -> u64 {
        if let Some(h) = self.cached_hash.get() {
            return h;
        }
        let mut hasher = crate::hash::Fnv1aHasher::new();
        for region in &self.regions {
            hasher.write(&region.bytes);
        }
        let h = hasher.finish();
        self.cached_hash.set(Some(h));
        h
    }

    /// Locate the region that entirely contains `[addr, addr+length)`.
    pub fn containing_region(&self, addr: u64, length: u64) -> Option<&Region> {
        let idx = self.regions.partition_point(|r| r.base() <= addr);
        if idx == 0 {
            return None;
        }
        let region = &self.regions[idx - 1];
        if region.contains(addr, length) {
            Some(region)
        } else {
            None
        }
    }

    /// Mutable counterpart to [`GuestMemory::containing_region`]. Used by
    /// [`GuestMemory::apply_commit`] to locate the write target in a single
    /// pass instead of resolving the region twice.
    fn containing_region_mut(&mut self, addr: u64, length: u64) -> Option<&mut Region> {
        let idx = self.regions.partition_point(|r| r.base() <= addr);
        if idx == 0 {
            return None;
        }
        let region = &mut self.regions[idx - 1];
        if region.contains(addr, length) {
            Some(region)
        } else {
            None
        }
    }

    /// Build a [`FaultContext`] for an out-of-region access at `addr`.
    pub fn fault_context(&self, addr: u64) -> FaultContext {
        let idx = self.regions.partition_point(|r| r.base() <= addr);
        let below = if idx > 0 {
            Some(self.regions[idx - 1].label())
        } else {
            None
        };
        let above = if idx < self.regions.len() {
            Some(self.regions[idx].label())
        } else {
            None
        };
        FaultContext {
            addr,
            nearest_below: below,
            nearest_above: above,
        }
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
        assert_eq!(mem.read(range(4, 4)).unwrap(), &[0, 0, 0, 0]);
    }

    #[test]
    fn commit_length_mismatch_rejected() {
        let mut mem = GuestMemory::new(16);
        let err = mem.apply_commit(range(0, 4), &[1, 2, 3]).unwrap_err();
        assert_eq!(err, MemError::LengthMismatch);
        assert_eq!(mem.read(range(0, 4)).unwrap(), &[0, 0, 0, 0]);
    }

    #[test]
    fn commit_out_of_range_rejected() {
        let mut mem = GuestMemory::new(8);
        let err = mem.apply_commit(range(6, 4), &[1, 2, 3, 4]).unwrap_err();
        assert!(matches!(err, MemError::Unmapped(_)));
        assert_eq!(mem.read(range(0, 8)).unwrap(), &[0; 8]);
    }

    #[test]
    fn read_checked_reports_unmapped_with_nearest_regions() {
        let mem = GuestMemory::from_regions(vec![
            Region::new(0, 0x100, "user_heap", PageSize::Page64K),
            Region::new(0x200, 0x100, "rsx", PageSize::Page64K),
        ])
        .unwrap();
        let err = mem.read_checked(range(0x150, 4)).unwrap_err();
        match err {
            MemError::Unmapped(ctx) => {
                assert_eq!(ctx.addr, 0x150);
                assert_eq!(ctx.nearest_below, Some("user_heap"));
                assert_eq!(ctx.nearest_above, Some("rsx"));
            }
            other => panic!("expected Unmapped, got {:?}", other),
        }
    }

    #[test]
    fn fault_context_no_regions_below_returns_none() {
        let mem =
            GuestMemory::from_regions(vec![Region::new(0x1000, 0x100, "heap", PageSize::Page64K)])
                .unwrap();
        let ctx = mem.fault_context(0x500);
        assert_eq!(ctx.addr, 0x500);
        assert_eq!(ctx.nearest_below, None);
        assert_eq!(ctx.nearest_above, Some("heap"));
    }

    #[test]
    fn fault_context_no_regions_above_returns_none() {
        let mem = GuestMemory::from_regions(vec![Region::new(0, 0x100, "heap", PageSize::Page64K)])
            .unwrap();
        let ctx = mem.fault_context(0x500);
        assert_eq!(ctx.addr, 0x500);
        assert_eq!(ctx.nearest_below, Some("heap"));
        assert_eq!(ctx.nearest_above, None);
    }

    #[test]
    fn containing_region_finds_matching_region() {
        let mem = GuestMemory::from_regions(vec![
            Region::new(0, 0x100, "a", PageSize::Page64K),
            Region::new(0x200, 0x100, "b", PageSize::Page64K),
        ])
        .unwrap();
        assert_eq!(mem.containing_region(0x50, 16).unwrap().label(), "a");
        assert_eq!(mem.containing_region(0x250, 16).unwrap().label(), "b");
        assert!(mem.containing_region(0x150, 16).is_none());
        assert!(mem.containing_region(0xF0, 0x20).is_none());
    }

    #[test]
    fn commit_zero_length_is_noop() {
        let mut mem = GuestMemory::new(8);
        mem.apply_commit(range(4, 0), &[]).unwrap();
        assert_eq!(mem.read(range(0, 8)).unwrap(), &[0; 8]);
    }

    #[test]
    fn overlapping_commits_apply_in_call_order() {
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
        let a = GuestMemory::new(8);
        let b = GuestMemory::new(16);
        assert_ne!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn content_hash_is_position_sensitive() {
        let mut a = GuestMemory::new(8);
        let mut b = GuestMemory::new(8);
        a.apply_commit(range(0, 1), &[0xff]).unwrap();
        b.apply_commit(range(4, 1), &[0xff]).unwrap();
        assert_ne!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn content_hash_round_trips_after_revert() {
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

    #[test]
    fn new_constructs_single_region_at_base_zero() {
        let mem = GuestMemory::new(16);
        let regions: Vec<_> = mem.regions().collect();
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].base(), 0);
        assert_eq!(regions[0].size(), 16);
        assert_eq!(regions[0].label(), "flat");
    }

    #[test]
    fn from_regions_empty_produces_unmapped_memory() {
        let mem = GuestMemory::from_regions(vec![]).unwrap();
        assert_eq!(mem.size(), 0);
        assert_eq!(mem.read(range(0, 1)), None);
    }

    #[test]
    fn from_regions_single_region_matches_new() {
        let a = GuestMemory::new(32);
        let b =
            GuestMemory::from_regions(vec![Region::new(0, 32, "flat", PageSize::Page64K)]).unwrap();
        assert_eq!(a.size(), b.size());
        assert_eq!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn from_regions_rejects_overlap_at_base() {
        let err = GuestMemory::from_regions(vec![
            Region::new(0, 0x100, "a", PageSize::Page64K),
            Region::new(0, 0x100, "b", PageSize::Page64K),
        ])
        .unwrap_err();
        assert_eq!(err, MemError::OverlappingRegions);
    }

    #[test]
    fn from_regions_rejects_partial_overlap() {
        let err = GuestMemory::from_regions(vec![
            Region::new(0, 0x100, "a", PageSize::Page64K),
            Region::new(0x80, 0x100, "b", PageSize::Page64K),
        ])
        .unwrap_err();
        assert_eq!(err, MemError::OverlappingRegions);
    }

    #[test]
    fn from_regions_rejects_containment() {
        let err = GuestMemory::from_regions(vec![
            Region::new(0, 0x200, "big", PageSize::Page64K),
            Region::new(0x80, 0x100, "small", PageSize::Page64K),
        ])
        .unwrap_err();
        assert_eq!(err, MemError::OverlappingRegions);
    }

    #[test]
    fn from_regions_accepts_adjacent_non_overlapping() {
        let mem = GuestMemory::from_regions(vec![
            Region::new(0, 0x100, "a", PageSize::Page64K),
            Region::new(0x100, 0x100, "b", PageSize::Page64K),
        ])
        .unwrap();
        assert_eq!(mem.regions().count(), 2);
    }

    #[test]
    fn reserved_zero_readable_region_reads_zero_and_bumps_counter() {
        let mem = GuestMemory::from_regions(vec![Region::with_access(
            0xC000_0000,
            0x100,
            "rsx",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        )])
        .unwrap();
        assert_eq!(mem.provisional_read_count(), 0);
        let bytes = mem.read(range(0xC000_0000, 8)).unwrap();
        assert_eq!(bytes, &[0u8; 8]);
        assert_eq!(mem.provisional_read_count(), 1);
        let _ = mem.read(range(0xC000_0040, 4)).unwrap();
        assert_eq!(mem.provisional_read_count(), 2);
    }

    #[test]
    fn reserved_zero_readable_region_writes_fault() {
        let mut mem = GuestMemory::from_regions(vec![Region::with_access(
            0xC000_0000,
            0x100,
            "rsx",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        )])
        .unwrap();
        let err = mem
            .apply_commit(range(0xC000_0000, 4), &[1, 2, 3, 4])
            .unwrap_err();
        assert!(matches!(err, MemError::ReservedWrite { region: "rsx", .. }));
    }

    #[test]
    fn reserved_strict_region_blocks_both_reads_and_writes() {
        let mut mem = GuestMemory::from_regions(vec![Region::with_access(
            0xE000_0000,
            0x100,
            "spu_reserved",
            PageSize::Page64K,
            RegionAccess::ReservedStrict,
        )])
        .unwrap();
        assert_eq!(mem.read(range(0xE000_0000, 4)), None);
        let err = mem
            .apply_commit(range(0xE000_0000, 4), &[1, 2, 3, 4])
            .unwrap_err();
        assert!(matches!(
            err,
            MemError::ReservedWrite {
                region: "spu_reserved",
                ..
            }
        ));
        assert_eq!(mem.provisional_read_count(), 0);
    }

    #[test]
    fn read_write_region_does_not_bump_provisional_counter() {
        let mem = GuestMemory::new(0x100);
        let _ = mem.read(range(0, 16));
        assert_eq!(mem.provisional_read_count(), 0);
    }

    #[test]
    fn multi_region_read_and_commit_route_by_address() {
        let mut mem = GuestMemory::from_regions(vec![
            Region::new(0, 0x100, "low", PageSize::Page64K),
            Region::new(0x1000, 0x100, "high", PageSize::Page64K),
        ])
        .unwrap();
        mem.apply_commit(range(0x10, 4), &[1, 2, 3, 4]).unwrap();
        mem.apply_commit(range(0x1010, 4), &[9, 9, 9, 9]).unwrap();
        assert_eq!(mem.read(range(0x10, 4)).unwrap(), &[1, 2, 3, 4]);
        assert_eq!(mem.read(range(0x1010, 4)).unwrap(), &[9, 9, 9, 9]);
        assert_eq!(mem.read(range(0x500, 4)), None);
    }

    #[test]
    fn read_checked_reports_strict_reserved_read() {
        let mem = GuestMemory::from_regions(vec![Region::with_access(
            0xE000_0000,
            0x100,
            "spu_reserved",
            PageSize::Page64K,
            RegionAccess::ReservedStrict,
        )])
        .unwrap();
        let err = mem.read_checked(range(0xE000_0000, 4)).unwrap_err();
        assert_eq!(
            err,
            MemError::ReservedStrictRead {
                addr: 0xE000_0000,
                region: "spu_reserved",
            }
        );
    }

    #[test]
    fn read_checked_bumps_provisional_counter_for_reserved_zero_readable() {
        let mem = GuestMemory::from_regions(vec![Region::with_access(
            0xC000_0000,
            0x100,
            "rsx",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        )])
        .unwrap();
        assert_eq!(mem.provisional_read_count(), 0);
        let bytes = mem.read_checked(range(0xC000_0000, 8)).unwrap();
        assert_eq!(bytes, &[0u8; 8]);
        assert_eq!(mem.provisional_read_count(), 1);
    }

    #[test]
    fn containing_region_rejects_straddle_across_adjacent_regions() {
        let mem = GuestMemory::from_regions(vec![
            Region::new(0, 0x100, "a", PageSize::Page64K),
            Region::new(0x100, 0x100, "b", PageSize::Page64K),
        ])
        .unwrap();
        assert!(mem.containing_region(0xF0, 0x20).is_none());
    }

    #[test]
    fn content_hash_is_idempotent_without_writes() {
        let mem = GuestMemory::new(16);
        let h1 = mem.content_hash();
        let h2 = mem.content_hash();
        let h3 = mem.content_hash();
        assert_eq!(h1, h2);
        assert_eq!(h2, h3);
    }
}
