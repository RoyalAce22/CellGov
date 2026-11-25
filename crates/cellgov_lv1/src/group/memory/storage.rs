//! The storage that holds the pages of the memory regions that
//! `allocate_memory` creates, and the answer of `write_htab_entry` for a
//! page in one of these regions.
//!
//! lv1 gives each page of a region a zero-filled page of its own memory,
//! at a real address that no call gives the partition; see
//! [`WRITE_HTAB_ALLOCATED_PAGE_SIZE_R3`]. CellGov keeps the pages in
//! guest storage at [`ALLOCATED_STORAGE_BASE`], so that the PPU reads
//! them through the page table. Each region that fits gets its own range
//! of the storage; see [`placement`].
//!
//! lv1's page allocator fills each page of a new allocation with 0
//! [OFW-4.93 lv1 a:0x2C14F8-0x2C1508].
//! CellGov gives a new region only storage that no region got before, so
//! the range of a new region holds 0 with no write. The storage of a
//! region that leaves the memory state is never given to another region.
//! When the storage has no room past the ranges that regions got, a new
//! region gets no storage, and `write_htab_entry` refuses its pages.

use cellgov_mem::{PageSize, Region};
use cellgov_ps3_abi::lv1::memory::WRITE_HTAB_ALLOCATED_PAGE_SIZE_R3;
use cellgov_ps3_abi::lv1::page_table::STORED_MEMORY_LIMIT;

use super::region::{MemoryRegion, MemoryState};

/// The real address of the storage that holds the pages of the allocated
/// regions.
///
/// CellGov chooses the address, with these properties:
/// - It lies above the storage of the shadow pages.
/// - It is a multiple of 1 MB, the largest page size of a region, so the
///   first region of the storage starts at the base.
/// - Each byte is at or below [`STORED_MEMORY_LIMIT`], so an entry that
///   maps a page of the storage stores the WIMG of memory, M.
///
/// The compiler checks the second and third properties.
pub const ALLOCATED_STORAGE_BASE: u64 = 0x1200_0000;

/// The size of the storage in bytes: the GameOS partition's memory past
/// its real mode region at 4.93.
///
/// This size is `bi.rgntotal` (0x0F9A_0000) less `rm_size` (0x0800_0000).
/// `allocate_memory` refuses a region past this memory, so each region it
/// creates fits, unless its page size alignment leaves a gap before it.
/// A region whose range does not fit in the storage gets no storage.
pub const ALLOCATED_STORAGE_BYTES: u64 = 0x079A_0000;

const _: () = assert!(ALLOCATED_STORAGE_BASE.is_multiple_of(1 << 20));
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
    let region = memory
        .regions()
        .iter()
        .find(|r| address >= r.lpar_address && address - r.lpar_address < r.size)?;
    if shift > u32::from(region.page_size_exp) {
        return Some(AllocatedPage::Lv1(WRITE_HTAB_ALLOCATED_PAGE_SIZE_R3));
    }
    Some(match memory.storage_of(region) {
        Some(real) => AllocatedPage::Stored(real + (address - region.lpar_address)),
        None => AllocatedPage::NoStorage,
    })
}

#[cfg(test)]
#[path = "tests/storage_tests.rs"]
mod tests;
