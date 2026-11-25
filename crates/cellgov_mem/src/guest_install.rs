//! Region installs, and the check that decides whether an install
//! succeeds.
//!
//! [`GuestMemory::check_install_region`] owns every reason an install
//! refuses a range. A caller that installs several regions as one unit
//! asks it for each range first, so a refusal installs nothing.

use super::{GuestMemory, MemError, PageHashes, PageSize, Region, RegionAccess, RESET_PAGE_SIZE};

impl GuestMemory {
    /// Insert a fresh zero-filled ReadWrite region.
    ///
    /// # Errors
    ///
    /// - [`MemError::OverlappingRegions`] when the new range overlaps a
    ///   region or an unmodelled span.
    /// - [`MemError::RegionOverflow`] when the new range runs past the end
    ///   of the address space.
    pub fn install_region(
        &mut self,
        base: u64,
        size: usize,
        label: &'static str,
        page_size: PageSize,
    ) -> Result<(), MemError> {
        self.install_region_with_access(base, size, label, page_size, RegionAccess::ReadWrite)
    }

    /// Insert a fresh zero-filled region with access mode `access`.
    ///
    /// # Errors
    ///
    /// As [`Self::install_region`].
    pub fn install_region_with_access(
        &mut self,
        base: u64,
        size: usize,
        label: &'static str,
        page_size: PageSize,
        access: RegionAccess,
    ) -> Result<(), MemError> {
        let insertion = self.install_slot(base, size, &[])?;
        self.regions.insert(
            insertion,
            Region::with_access(base, size, label, page_size, access),
        );
        self.page_hashes
            .get_mut()
            .insert(insertion, PageHashes::new(size.div_ceil(RESET_PAGE_SIZE)));
        self.cached_hash.set(None);
        Ok(())
    }

    /// Check that [`Self::install_region`] accepts `[base, base + size)`
    /// after the installs of the `pending` ranges, and change nothing.
    ///
    /// Each `pending` entry is a `(base, size)` pair that this check
    /// accepted before, with the entries before it as its own `pending`.
    ///
    /// # Errors
    ///
    /// The error that [`Self::install_region`] returns for the range, with
    /// each `pending` range counted as a region.
    pub fn check_install_region(
        &self,
        base: u64,
        size: usize,
        pending: &[(u64, usize)],
    ) -> Result<(), MemError> {
        self.install_slot(base, size, pending).map(|_| ())
    }

    /// The index in `regions` where an install of `[base, base + size)`
    /// goes, or the reason the install refuses the range.
    fn install_slot(
        &self,
        base: u64,
        size: usize,
        pending: &[(u64, usize)],
    ) -> Result<usize, MemError> {
        let new_end = u128::from(base) + size as u128;
        if new_end > u128::from(u64::MAX) {
            return Err(MemError::RegionOverflow {
                base,
                size: size as u64,
            });
        }
        // The regions do not overlap and are in base order. So the region
        // before the slot and the region after it are the only regions
        // that can refuse the range.
        let insertion = self.regions.partition_point(|r| r.base() <= base);
        let neighbours = insertion
            .checked_sub(1)
            .into_iter()
            .chain(Some(insertion))
            .filter_map(|i| self.regions.get(i))
            .map(|r| (r.base(), u128::from(r.base()) + u128::from(r.size())));
        let pending = pending.iter().map(|&(other_base, other_size)| {
            (other_base, u128::from(other_base) + other_size as u128)
        });
        let blocked = neighbours
            .chain(pending)
            .any(|(other_base, other_end)| refuses(other_base, other_end, base, new_end));
        if blocked {
            return Err(MemError::OverlappingRegions);
        }
        // An unmodelled span holds no bytes, so no region may give it some.
        let spanned = self
            .unmodelled_spans
            .iter()
            .any(|&(lo, hi)| u128::from(lo) < new_end && base < hi);
        if spanned {
            return Err(MemError::OverlappingRegions);
        }
        Ok(insertion)
    }
}

/// Whether a region over `[other_base, other_end)` refuses an install
/// over `[base, new_end)`.
///
/// A region at or below `base` refuses when it reaches past `base`. So a
/// region refuses a zero-size install at its own base. A region above
/// `base` refuses when the install reaches past its base.
fn refuses(other_base: u64, other_end: u128, base: u64, new_end: u128) -> bool {
    if other_base <= base {
        other_end > u128::from(base)
    } else {
        new_end > u128::from(other_base)
    }
}
