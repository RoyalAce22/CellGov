//! Unmodelled spans: real addresses where the machine has storage that
//! CellGov does not model, such as a device's MMIO registers.
//!
//! A span holds no bytes. A load, a store, a fetch and the commit
//! pipeline find no region there, as at an address no storage holds, so
//! a span never reads as zeros and a write there never lands. Only
//! [`GuestMemory::is_unmodelled`] tells the two cases apart. An execution
//! unit asks it to stop at the access as an unmodelled stop, where an
//! address no storage holds takes the processor's own fault.
//!
//! A span is not guest state: only a loader declares one, so the content
//! hash leaves the spans out.

use super::{GuestMemory, MemError};

impl GuestMemory {
    /// Record `[base, base + len)` as an unmodelled span. A `len` of 0
    /// records nothing.
    ///
    /// # Errors
    ///
    /// Returns one of these, and records nothing:
    /// - [`MemError::RegionOverflow`] when the span runs past the end of
    ///   the address space;
    /// - [`MemError::OverlappingRegions`] when the span overlaps a region
    ///   or a recorded span.
    pub fn declare_unmodelled(&mut self, base: u64, len: u64) -> Result<(), MemError> {
        let end = base
            .checked_add(len)
            .ok_or(MemError::RegionOverflow { base, size: len })?;
        if len == 0 {
            return Ok(());
        }
        let overlaps = |(lo, hi): (u64, u64)| lo < end && base < hi;
        let regions = self.regions.iter().map(|r| (r.base(), r.end()));
        if regions
            .chain(self.unmodelled_spans.iter().copied())
            .any(overlaps)
        {
            return Err(MemError::OverlappingRegions);
        }
        let at = self.unmodelled_spans.partition_point(|&(lo, _)| lo < base);
        self.unmodelled_spans.insert(at, (base, end));
        Ok(())
    }

    /// Whether an unmodelled span holds the byte at `addr`.
    pub fn is_unmodelled(&self, addr: u64) -> bool {
        let next = self.unmodelled_spans.partition_point(|&(lo, _)| lo <= addr);
        next.checked_sub(1)
            .is_some_and(|i| addr < self.unmodelled_spans[i].1)
    }

    /// The unmodelled spans `[start, end)`, in address order.
    pub fn unmodelled_spans(&self) -> &[(u64, u64)] {
        &self.unmodelled_spans
    }
}
