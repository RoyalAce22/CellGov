//! The storage that holds the pages of the memory regions that
//! `allocate_memory` creates, and the answer of `write_htab_entry` for a
//! page in one of these regions.
//!
//! lv1 gives each page of a region a zero-filled page of its own memory,
//! at a real address that no call gives the partition; see
//! [`WRITE_HTAB_ALLOCATED_PAGE_SIZE_R3`]. CellGov keeps the pages in
//! guest storage at [`ALLOCATED_STORAGE_BASE`], so that the PPU reads
//! them through the page table. Each region gets the next range of the
//! storage, aligned to its page size, when the range fits. No two regions
//! get the same byte, so each range holds 0, as lv1's pages do after the
//! allocation.

use cellgov_mem::{PageSize, Region};
use cellgov_ps3_abi::lv1::memory::WRITE_HTAB_ALLOCATED_PAGE_SIZE_R3;
use cellgov_ps3_abi::lv1::page_table::STORED_MEMORY_LIMIT;

use super::region::{MemoryRegion, MemoryState};

/// The real address of the storage that holds the pages of the allocated
/// regions.
///
/// CellGov chooses the address, with these properties:
/// - It lies above the storage of the shadow pages.
/// - Each byte is at or below [`STORED_MEMORY_LIMIT`], so an entry that
///   maps a page of the storage stores the WIMG of memory, M.
///
/// The compiler checks the second property.
pub const ALLOCATED_STORAGE_BASE: u64 = 0x1200_0000;

/// The size of the storage in bytes. CellGov chooses the size; a region
/// past it gets no storage.
pub const ALLOCATED_STORAGE_BYTES: u64 = 16 << 20;

const _: () = assert!(ALLOCATED_STORAGE_BASE + ALLOCATED_STORAGE_BYTES - 1 <= STORED_MEMORY_LIMIT);

/// The region that holds the pages of the allocated regions, for a boot
/// that runs the kernel under CellGov's hypervisor.
pub fn allocated_storage() -> Region {
    Region::new(
        ALLOCATED_STORAGE_BASE,
        ALLOCATED_STORAGE_BYTES as usize,
        "lv1 allocated memory",
        PageSize::Page64K,
    )
}

/// The real address of the range that `region` gets after the ranges that
/// end at `end`, or `None` when the range does not fit in the storage.
///
/// The range starts at the first multiple of the region's page size at or
/// above `end`.
pub(crate) fn placement(end: u64, region: &MemoryRegion) -> Option<u64> {
    let page = 1u64.checked_shl(u32::from(region.page_size_exp))?;
    let start = end.checked_next_multiple_of(page)?;
    let fits = start
        .checked_add(region.size)
        .is_some_and(|stop| stop <= ALLOCATED_STORAGE_BASE + ALLOCATED_STORAGE_BYTES);
    fits.then_some(start)
}

/// The answer of `write_htab_entry` for a valid entry whose page lies in
/// an allocated region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AllocatedPage {
    /// The page lies in the region's storage, at this real address.
    Stored(u64),
    /// lv1's translation refuses the entry with this r3.
    Lv1(u64),
    /// The region gets no storage, so CellGov refuses the entry.
    NoStorage,
}

/// The answer for a valid entry whose page of 2^`shift` bytes lies at the
/// logical partition address `address`.
///
/// The answer is `None` when no region in `memory` holds the address. lv1
/// finds the region by the page's address alone. Its translation checks
/// the page's size and does not read the entry's page protection; see
/// [`WRITE_HTAB_ALLOCATED_PAGE_SIZE_R3`].
pub(crate) fn region_page(memory: &MemoryState, address: u64, shift: u32) -> Option<AllocatedPage> {
    let (index, region) = (0u64..)
        .zip(memory.regions())
        .find(|(_, r)| address >= r.lpar_address && address - r.lpar_address < r.size)?;
    if shift > u32::from(region.page_size_exp) {
        return Some(AllocatedPage::Lv1(WRITE_HTAB_ALLOCATED_PAGE_SIZE_R3));
    }
    Some(match memory.storage_real(index) {
        Some(real) => AllocatedPage::Stored(real + (address - region.lpar_address)),
        None => AllocatedPage::NoStorage,
    })
}

#[cfg(test)]
#[path = "tests/storage_tests.rs"]
mod tests;
