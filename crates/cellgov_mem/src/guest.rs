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
//! Backing layout: a `BTreeMap<u64, Region>` keyed by region base
//! address. Each [`Region`] owns a `Vec<u8>` sized to that region and
//! carries a label and page-size class. Region lookup is `O(log n)`; the
//! region count stays single-digit in current usage, so the constant
//! factor dominates and the scan is predictable.
//!
//! [`GuestMemory::new`] is a convenience constructor for the common
//! single-region case: one region at base 0 spanning `[0, size)`.
//! Multi-region layouts use [`GuestMemory::from_regions`].

use std::cell::Cell;
use std::collections::BTreeMap;

use crate::range::ByteRange;

/// Page-size class of a region. Informational metadata; the region
/// map does not implement page-granular protection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PageSize {
    /// 4 KB pages. Standard Linux/PS3 small-page size.
    Page4K,
    /// 64 KB pages. Default for PS3 LV2 user memory.
    Page64K,
    /// 1 MB pages. Used for large allocations on PS3.
    Page1M,
}

/// Access mode of a region.
///
/// Three states are encoded explicitly so the provisional
/// zero-readable behavior of an unimplemented region stays separable
/// from a real read-write region. The fault type surfaced by a
/// wrong-mode access names which mode it tripped, so a "tried to
/// write to RSX" diagnostic is distinct from a normal out-of-region
/// fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionAccess {
    /// Normal user-memory region: reads return the stored bytes,
    /// writes via `apply_commit` succeed.
    ReadWrite,
    /// Reserved provisional region: reads return zero (the region's
    /// zero-init backing), writes fault. Used as a placeholder for
    /// regions whose semantics (RSX local memory, SPU shared) are
    /// not yet implemented. Reads bump a counter on `GuestMemory` so
    /// silent zero-reads surface in the divergence finder.
    ReservedZeroReadable,
    /// Reserved strict region: reads and writes both fault. Used by
    /// tests asserting no code paths touch the region yet.
    ReservedStrict,
}

/// A single contiguous guest memory region.
///
/// Regions are the unit of address-space reservation. Each region knows
/// its base, size, label, page-size class, and access mode;
/// out-of-region addresses fault before touching any backing store.
#[derive(Debug, Clone)]
pub struct Region {
    base: u64,
    bytes: Vec<u8>,
    label: &'static str,
    page_size: PageSize,
    access: RegionAccess,
}

impl Region {
    /// Construct a normal read-write region (zero-filled).
    #[inline]
    pub fn new(base: u64, size: usize, label: &'static str, page_size: PageSize) -> Self {
        Self::with_access(base, size, label, page_size, RegionAccess::ReadWrite)
    }

    /// Construct a region with an explicit access mode.
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

    /// Human-readable label used in diagnostics (e.g. `"flat"`,
    /// `"user_heap"`, `"stack"`).
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

    /// End address (exclusive). Saturates at `u64::MAX` for regions that
    /// would otherwise wrap.
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
/// Construct with [`GuestMemory::new`] for a single flat region
/// spanning `[0, size)`, or [`GuestMemory::from_regions`] for a sparse
/// multi-region layout. Reads are checked against the region map;
/// out-of-region reads return `None` rather than panicking, since
/// reading past a region boundary is a guest-induced fault, not a
/// runtime invariant violation.
#[derive(Debug, Clone)]
pub struct GuestMemory {
    regions: BTreeMap<u64, Region>,
    /// Cached content hash. `None` means the cache is stale (a write
    /// happened since the last hash computation). Uses `Cell` for
    /// interior mutability so `content_hash` can stay `&self`.
    cached_hash: Cell<Option<u64>>,
    /// Count of reads that landed in a `ReservedZeroReadable` region
    /// since construction. Bumped by every successful read whose
    /// target is provisional. Downstream diagnostics read this counter
    /// to surface silent zero-reads from RSX/SPU placeholder regions
    /// rather than letting them pass unnoticed.
    provisional_read_count: Cell<u64>,
}

/// Diagnostic context for an out-of-region access.
///
/// Produced by [`GuestMemory::fault_context`] and carried by
/// [`MemError::Unmapped`]. Names the faulting address and the labels
/// of the nearest mapped regions below and above it, so a diagnostic
/// at `0xB0000000` can say "between `user_heap` and `rsx`" rather
/// than just "out of bounds".
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
///
/// `MemError` is local to `cellgov_mem`; there is no universal `Error`
/// enum spanning the workspace. Boundary crates that consume
/// these errors -- the commit pipeline in `cellgov_core` and the
/// validation layer in `cellgov_effects` -- convert into their own
/// error types at the crate boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemError {
    /// The requested range is ill-formed (length overflows `u64`) or
    /// not usable for a region-map lookup for reasons other than
    /// being in an unmapped range. Kept for local-store and staging
    /// call sites that do not carry a `FaultContext`.
    OutOfRange,
    /// The supplied byte buffer's length does not match the range's
    /// length. Only relevant for [`GuestMemory::apply_commit`].
    LengthMismatch,
    /// `from_regions` was called with two regions whose address ranges
    /// overlap.
    OverlappingRegions,
    /// The requested range is not entirely contained within any single
    /// mapped region. The payload names the faulting address and the
    /// nearest regions on either side for diagnostics.
    Unmapped(FaultContext),
    /// Write attempted to a reserved region (RSX, SPU-shared, or any
    /// region explicitly declared non-writable). Names the region
    /// label for diagnostics.
    ReservedWrite {
        /// Guest address of the faulting write.
        addr: u64,
        /// Label of the reserved region the write targeted.
        region: &'static str,
    },
    /// Read attempted to a reserved region in strict mode. Distinct
    /// from `Unmapped` so tests can tell "tried to read RSX while
    /// strict-reserved was on" from "address not mapped at all".
    ReservedStrictRead {
        /// Guest address of the faulting read.
        addr: u64,
        /// Label of the reserved region the read targeted.
        region: &'static str,
    },
}

impl GuestMemory {
    /// Construct a fresh `GuestMemory` with a single region at base 0.
    ///
    /// Convenience constructor for the common single-region case:
    /// one contiguous region starting at address 0. Tests and
    /// scenarios that do not need a sparse multi-region layout use
    /// this entry point.
    ///
    /// Takes a `usize` rather than a `u64` because the backing is a
    /// `Vec<u8>` and `Vec` is `usize`-indexed. Multi-region layouts
    /// that span the full PS3 64-bit address space use
    /// [`GuestMemory::from_regions`].
    #[inline]
    pub fn new(size: usize) -> Self {
        // Single region at base 0 can never overlap with itself, so the
        // construction is infallible.
        Self::from_regions(vec![Region::new(0, size, "flat", PageSize::Page64K)])
            .expect("single region at base 0 is always non-overlapping")
    }

    /// Construct `GuestMemory` from a set of regions.
    ///
    /// Returns `Err(MemError::OverlappingRegions)` if any two regions'
    /// address ranges overlap. Empty input is allowed and produces a
    /// `GuestMemory` with no mapped addresses (every read faults).
    pub fn from_regions(regions: Vec<Region>) -> Result<Self, MemError> {
        let mut map: BTreeMap<u64, Region> = BTreeMap::new();
        for region in regions {
            let base = region.base();
            let end = region.end();
            // Overlap check: no existing region's [base, end) may
            // intersect [region.base, region.end). Two ranges
            // [a, b) and [c, d) overlap iff a < d AND c < b.
            for existing in map.values() {
                let ex_base = existing.base();
                let ex_end = existing.end();
                if base < ex_end && ex_base < end {
                    return Err(MemError::OverlappingRegions);
                }
            }
            map.insert(base, region);
        }
        Ok(Self {
            regions: map,
            cached_hash: Cell::new(None),
            provisional_read_count: Cell::new(0),
        })
    }

    /// Number of reads that have landed in a `ReservedZeroReadable`
    /// region since construction.
    #[inline]
    pub fn provisional_read_count(&self) -> u64 {
        self.provisional_read_count.get()
    }

    /// Total backing size in bytes (sum of every region's size).
    ///
    /// For a `new(size)` construction this equals `size`. For
    /// multi-region layouts this is the sum of mapped bytes, not the
    /// address-space span (which would include gaps between regions).
    #[inline]
    pub fn size(&self) -> u64 {
        self.regions.values().map(|r| r.size()).sum()
    }

    /// All regions in address order.
    pub fn regions(&self) -> impl Iterator<Item = &Region> {
        self.regions.values()
    }

    /// Read the primary region at base 0 as a byte slice.
    ///
    /// Convenience accessor for the base-0 region. In multi-region
    /// layouts this returns only the region at base 0 (the "main"
    /// region holding the loaded ELF and the user-memory allocator).
    /// Auxiliary regions (stack at `0xD0000000+`, reserved RSX/SPU
    /// ranges) are not visible through this accessor and must be
    /// reached via [`GuestMemory::read`] or by iterating
    /// [`GuestMemory::regions`].
    ///
    /// Returns an empty slice if no region exists at base 0.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        match self.regions.get(&0) {
            Some(r) => &r.bytes,
            None => &[],
        }
    }

    /// Read the bytes covered by `range`.
    ///
    /// Returns `None` if the range is not entirely contained within a
    /// single region. Zero-length ranges whose start address is mapped
    /// (or exactly at the end of a region) return an empty slice.
    ///
    /// For diagnostic-rich failure information, use
    /// [`GuestMemory::read_checked`].
    pub fn read(&self, range: ByteRange) -> Option<&[u8]> {
        let start = range.start().raw();
        let length = range.length();
        let end = start.checked_add(length)?;
        let region = self.containing_region(start, length)?;
        // ReservedStrict regions deny reads. ReservedZeroReadable
        // regions allow reads but bump the provisional counter so the
        // silent zeros surface in `run-game`'s end-of-boot summary.
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

    /// Read the bytes covered by `range`, returning a typed error on
    /// failure.
    ///
    /// Returns [`MemError::Unmapped`] with a [`FaultContext`] naming
    /// the nearest regions when the range is not contained by any
    /// region; returns [`MemError::OutOfRange`] for length overflow.
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

    /// Apply a committed write to `range` from `bytes`.
    ///
    /// **This is the commit pipeline's entry point and should not be
    /// called from execution units.** No execution unit may publish
    /// guest-visible state directly; this method exists only so that
    /// `cellgov_core`'s commit applier has a typed entry point to
    /// call. Nothing enforces this at the language level -- the rule
    /// is architectural -- but the entry point is narrow enough that
    /// violations are visible at code review.
    ///
    /// Returns `Err(MemError::LengthMismatch)` if `bytes.len() as u64
    /// != range.length()`, and `Err(MemError::Unmapped(ctx))` if the
    /// range is not entirely contained within a single region (with
    /// `ctx` naming the nearest mapped regions on either side).
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
        let region = self
            .regions
            .get_mut(&base)
            .expect("just located via lookup");
        let offset = start - region.base();
        let offset_usize = usize::try_from(offset).map_err(|_| MemError::OutOfRange)?;
        let end_offset = offset + length;
        let end_usize = usize::try_from(end_offset).map_err(|_| MemError::OutOfRange)?;
        region.bytes[offset_usize..end_usize].copy_from_slice(bytes);
        self.cached_hash.set(None);
        Ok(())
    }

    /// 64-bit content hash of every committed byte, in address order.
    ///
    /// FNV-1a, no external deps, no host-specific seeding. Checkpoint
    /// hashes must be deterministic across hosts and runs; FNV-1a
    /// satisfies that and is small enough to inline. Replay tooling
    /// compares pairs of these values to assert that two runs reached
    /// bit-identical committed-memory state. Regions are hashed in
    /// address order so the digest stays stable across clones and
    /// across runs that build the region set in the same order.
    pub fn content_hash(&self) -> u64 {
        if let Some(h) = self.cached_hash.get() {
            return h;
        }
        let mut hasher = crate::hash::Fnv1aHasher::new();
        for region in self.regions.values() {
            hasher.write(&region.bytes);
        }
        let h = hasher.finish();
        self.cached_hash.set(Some(h));
        h
    }

    /// Locate the region that entirely contains `[addr, addr+length)`,
    /// if any. Public so callers (staging, local validation) can
    /// pre-check containment without triggering an error path.
    pub fn containing_region(&self, addr: u64, length: u64) -> Option<&Region> {
        // Largest region whose base <= addr, via BTreeMap range.
        let (_, region) = self.regions.range(..=addr).next_back()?;
        if region.contains(addr, length) {
            Some(region)
        } else {
            None
        }
    }

    /// Diagnostic context for an out-of-region access at `addr`.
    ///
    /// Names the addresses of the nearest mapped regions below and
    /// above `addr`. Callers should invoke this when constructing a
    /// fault diagnostic after [`GuestMemory::containing_region`]
    /// returns `None`.
    pub fn fault_context(&self, addr: u64) -> FaultContext {
        // Nearest-below: largest region with base <= addr.
        let below = self
            .regions
            .range(..=addr)
            .next_back()
            .map(|(_, r)| r.label());
        // Nearest-above: smallest region with base > addr.
        let above = self
            .regions
            .range((addr.saturating_add(1))..)
            .next()
            .map(|(_, r)| r.label());
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
