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
    /// Cached FNV-1a digest; `None` iff a write has happened since the last
    /// computation. `Cell` lets [`GuestMemory::content_hash`] take `&self`.
    cached_hash: Cell<Option<u64>>,
    /// Monotonic count of reads that hit a [`RegionAccess::ReservedZeroReadable`]
    /// region. Diagnostics surface silent zero-reads via this counter.
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
    /// Range is ill-formed (end overflows `u64`) or the length does not
    /// fit `usize`. Used by call sites that do not carry a `FaultContext`.
    OutOfRange,
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

    /// Reads that have hit a `ReservedZeroReadable` region since construction.
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

    /// Read the bytes covered by `range`, or `None` if it is not entirely
    /// contained within a single region.
    ///
    /// A read from a `ReservedZeroReadable` region bumps
    /// [`GuestMemory::provisional_read_count`]. For diagnostic-rich failure
    /// information, use [`GuestMemory::read_checked`].
    pub fn read(&self, range: ByteRange) -> Option<&[u8]> {
        let start = range.start().raw();
        let length = range.length();
        let end = start.checked_add(length)?;
        let region = self.containing_region(start, length)?;
        match region.access {
            RegionAccess::ReadWrite => {}
            RegionAccess::ReservedZeroReadable => {
                self.provisional_read_count
                    .set(self.provisional_read_count.get() + 1);
            }
            RegionAccess::ReservedStrict => return None,
        }
        let offset = start - region.base();
        let offset_usize = usize::try_from(offset).ok()?;
        let end_offset = offset + (end - start);
        let end_usize = usize::try_from(end_offset).ok()?;
        Some(&region.bytes[offset_usize..end_usize])
    }

    /// Read the bytes covered by `range`, returning a typed error on failure.
    ///
    /// # Errors
    ///
    /// - [`MemError::OutOfRange`] on length overflow.
    /// - [`MemError::Unmapped`] with a [`FaultContext`] when no single region
    ///   contains the range.
    pub fn read_checked(&self, range: ByteRange) -> Result<&[u8], MemError> {
        let start = range.start().raw();
        let length = range.length();
        let _end = start.checked_add(length).ok_or(MemError::OutOfRange)?;
        let region = self
            .containing_region(start, length)
            .ok_or_else(|| MemError::Unmapped(self.fault_context(start)))?;
        let offset = start - region.base();
        let offset_usize = usize::try_from(offset).map_err(|_| MemError::OutOfRange)?;
        let end_offset = offset + length;
        let end_usize = usize::try_from(end_offset).map_err(|_| MemError::OutOfRange)?;
        Ok(&region.bytes[offset_usize..end_usize])
    }

    /// Apply a committed write to `range` from `bytes`. Invalidates the
    /// cached content hash.
    ///
    /// This is the commit pipeline's entry point; execution units must not
    /// call it directly. The rule is architectural, not language-enforced.
    ///
    /// # Errors
    ///
    /// - [`MemError::LengthMismatch`] if `bytes.len() as u64 != range.length()`.
    /// - [`MemError::Unmapped`] if no single region contains the range.
    /// - [`MemError::ReservedWrite`] if the target region is not `ReadWrite`.
    pub fn apply_commit(&mut self, range: ByteRange, bytes: &[u8]) -> Result<(), MemError> {
        if bytes.len() as u64 != range.length() {
            return Err(MemError::LengthMismatch);
        }
        let start = range.start().raw();
        let length = range.length();
        let _end = start.checked_add(length).ok_or(MemError::OutOfRange)?;
        let (base, access, label) = match self.containing_region(start, length) {
            Some(r) => (r.base(), r.access(), r.label()),
            None => return Err(MemError::Unmapped(self.fault_context(start))),
        };
        if access != RegionAccess::ReadWrite {
            return Err(MemError::ReservedWrite {
                addr: start,
                region: label,
            });
        }
        let idx = self.regions.partition_point(|r| r.base() < base);
        let region = &mut self.regions[idx];
        let offset = start - region.base();
        let offset_usize = usize::try_from(offset).map_err(|_| MemError::OutOfRange)?;
        let end_offset = offset + length;
        let end_usize = usize::try_from(end_offset).map_err(|_| MemError::OutOfRange)?;
        region.bytes[offset_usize..end_usize].copy_from_slice(bytes);
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
#[path = "tests/guest_tests.rs"]
mod tests;
