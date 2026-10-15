//! Windows carved from a reserved region: a range of a reserved region
//! that becomes a `ReadWrite` region of its own, and returns to the
//! reserved region later.
//!
//! A Raw SPU's problem-state window is one. The reserved region stays
//! whole in every memory that maps no window, so its region map and
//! the content hash over it change only while the memory holds a window.

use super::{GuestMemory, MemError, PageHashes, Region, RegionAccess, RESET_PAGE_SIZE};

impl GuestMemory {
    /// Give the bytes `[base, base + size)` a zero-filled `ReadWrite`
    /// region of their own, labelled `label`.
    ///
    /// The bytes must lie inside one reserved region, which no write
    /// reaches, so its bytes are all zero. This call splits that region
    /// around the window. Each piece keeps its label, page size and access.
    ///
    /// # Errors
    ///
    /// - [`MemError::Unmapped`] when no single region holds the range.
    /// - [`MemError::OverlappingRegions`] when the region that holds it
    ///   is `ReadWrite`.
    pub fn map_window(
        &mut self,
        base: u64,
        size: usize,
        label: &'static str,
    ) -> Result<(), MemError> {
        let idx = self
            .containing_region_index(base, size as u64)
            .ok_or_else(|| MemError::Unmapped(self.fault_context(base)))?;
        let host = &self.regions[idx];
        if host.access == RegionAccess::ReadWrite {
            return Err(MemError::OverlappingRegions);
        }
        let (host_base, host_end) = (host.base, host.end());
        let (host_label, page_size, access) = (host.label, host.page_size, host.access);
        let end = base + size as u64;
        let mut pieces = Vec::with_capacity(3);
        if host_base < base {
            pieces.push(Region::with_access(
                host_base,
                (base - host_base) as usize,
                host_label,
                page_size,
                access,
            ));
        }
        let mut window = Region::with_access(base, size, label, page_size, RegionAccess::ReadWrite);
        window.window = Some((host_label, access));
        pieces.push(window);
        if end < host_end {
            pieces.push(Region::with_access(
                end,
                (host_end - end) as usize,
                host_label,
                page_size,
                access,
            ));
        }
        self.replace_regions(idx..idx + 1, pieces);
        Ok(())
    }

    /// Return the window at `base` to the reserved region it came from.
    ///
    /// The window:
    ///
    /// - goes back to zero bytes
    /// - takes the label and access of that region again
    /// - joins each adjacent piece of that region
    ///
    /// A window whose neighbours are both other windows stays a piece of
    /// its own until they return too.
    ///
    /// # Errors
    ///
    /// [`MemError::Unmapped`] when no window starts at `base`.
    pub fn unmap_window(&mut self, base: u64) -> Result<(), MemError> {
        let Some((idx, (label, access))) = self
            .regions
            .iter()
            .enumerate()
            .find_map(|(i, r)| r.window.filter(|_| r.base == base).map(|host| (i, host)))
        else {
            return Err(MemError::Unmapped(self.fault_context(base)));
        };
        let window = &self.regions[idx];
        let (window_base, window_end, page_size) = (window.base, window.end(), window.page_size);
        // Raw SPU windows sit side by side, so a neighbour can be another
        // window: only a non-window piece of the same reserved region joins.
        let joins = |other: &Region| {
            other.window.is_none()
                && other.label == label
                && other.access == access
                && other.page_size == page_size
        };
        let first = idx
            .checked_sub(1)
            .filter(|&i| self.regions[i].end() == window_base && joins(&self.regions[i]))
            .unwrap_or(idx);
        let last = Some(idx + 1)
            .filter(|&i| {
                i < self.regions.len()
                    && self.regions[i].base == window_end
                    && joins(&self.regions[i])
            })
            .unwrap_or(idx);
        let joined_base = self.regions[first].base;
        let joined_end = self.regions[last].end();
        let joined = Region::with_access(
            joined_base,
            (joined_end - joined_base) as usize,
            label,
            page_size,
            access,
        );
        self.replace_regions(first..last + 1, vec![joined]);
        Ok(())
    }

    /// Replace the regions in `range` with `pieces`, which cover the same
    /// addresses, and their page hashes with fresh ones.
    fn replace_regions(&mut self, range: std::ops::Range<usize>, pieces: Vec<Region>) {
        let hashes: Vec<PageHashes> = pieces
            .iter()
            .map(|r| PageHashes::new(r.bytes.len().div_ceil(RESET_PAGE_SIZE)))
            .collect();
        self.page_hashes.get_mut().splice(range.clone(), hashes);
        self.regions.splice(range, pieces);
        self.cached_hash.set(None);
    }
}

#[cfg(test)]
#[path = "tests/guest_window_tests.rs"]
mod tests;
