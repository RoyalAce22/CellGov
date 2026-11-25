//! The storage that holds the shadow pages of the logical SPEs, and the
//! answer of `write_htab_entry` for a page in a logical SPE's regions.
//!
//! lv1 backs each shadow page with a zero-filled page of its own memory,
//! at a real address that no call gives the partition; see
//! [`cellgov_ps3_abi::lv1::spe`]. CellGov keeps the pages in guest
//! storage at [`SHADOW_STORAGE_BASE`], so that the PPU reads them
//! through the page table. Each SPE gets the next page of the storage,
//! and no page is given twice. So each page that an SPE gets holds 0, as
//! lv1's page does after the construct.

use cellgov_mem::{PageSize, Region};
use cellgov_ps3_abi::lv1::page_table::{read_only, STORED_MEMORY_LIMIT};
use cellgov_ps3_abi::lv1::spe::{
    LOGICAL_SPE_AREA_SIZE, WRITE_HTAB_SHADOW_PAGE_SIZE_R3, WRITE_HTAB_SHADOW_WRITABLE_R3,
};

use super::construct::{LogicalSpeState, SHADOW_PAGE_SHIFT};
use crate::group::memory::page_table::{PAGE_TABLE_STORAGE_BASE, PAGE_TABLE_STORAGE_BYTES};

/// The real address of the storage that holds the shadow pages.
///
/// CellGov chooses the address, with these properties:
/// - It lies above the hypervisor's page table storage.
/// - Each page is at or below [`STORED_MEMORY_LIMIT`], so an entry that
///   maps a shadow page stores the WIMG of memory, M.
///
/// The compiler checks both properties.
pub const SHADOW_STORAGE_BASE: u64 = 0x1100_0000;

/// The number of shadow pages that the storage holds. CellGov chooses the
/// number; a construct past it is a refusal.
pub const SHADOW_STORAGE_PAGES: u64 = 16;

const _: () = assert!(PAGE_TABLE_STORAGE_BASE + PAGE_TABLE_STORAGE_BYTES <= SHADOW_STORAGE_BASE);
const _: () = assert!(
    SHADOW_STORAGE_BASE + (SHADOW_STORAGE_PAGES << SHADOW_PAGE_SHIFT) - 1 <= STORED_MEMORY_LIMIT
);

/// The region that holds the shadow pages, for a boot that runs the
/// kernel under CellGov's hypervisor.
pub fn shadow_storage() -> Region {
    Region::new(
        SHADOW_STORAGE_BASE,
        (SHADOW_STORAGE_PAGES << SHADOW_PAGE_SHIFT) as usize,
        "lv1 shadow pages",
        PageSize::Page4K,
    )
}

/// The real address of shadow page `index` of the storage, or `None`
/// when the storage holds no such page.
pub(crate) fn shadow_page_real(index: u64) -> Option<u64> {
    (index < SHADOW_STORAGE_PAGES).then(|| SHADOW_STORAGE_BASE + (index << SHADOW_PAGE_SHIFT))
}

/// The answer of `write_htab_entry` for a valid entry whose page lies in
/// a region of a logical SPE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpeRegionPage {
    /// The page lies in a shadow page, at this real address.
    Shadow(u64),
    /// lv1's translation refuses the entry with this r3.
    Lv1(u64),
    /// CellGov refuses the entry for this reason.
    Refused(&'static str),
}

/// The answer for a valid entry with the doubleword 1 `doubleword1`,
/// whose page at the logical partition address `address` holds
/// 2^`shift` bytes, or `None` when no region of an SPE in `spes` holds
/// the address.
///
/// lv1 finds the region by the page's address alone, and its translation
/// checks the page's size; see [`cellgov_ps3_abi::lv1::spe`]. The
/// translation of an SPE's area is not read, so a page in an area is a
/// refusal.
pub(crate) fn region_page(
    spes: &LogicalSpeState,
    address: u64,
    shift: u32,
    doubleword1: u64,
) -> Option<SpeRegionPage> {
    let inside = |base: u64, size: u64| address >= base && address - base < size;
    let shadow_size = 1u64 << SHADOW_PAGE_SHIFT;
    for spe in spes.spes().values() {
        if inside(spe.area, LOGICAL_SPE_AREA_SIZE) {
            return Some(SpeRegionPage::Refused(
                "a valid entry whose page lies in a logical SPE's area",
            ));
        }
        if !inside(spe.shadow, shadow_size) {
            continue;
        }
        return Some(if shift > SHADOW_PAGE_SHIFT {
            SpeRegionPage::Lv1(WRITE_HTAB_SHADOW_PAGE_SIZE_R3)
        } else if !read_only(doubleword1) {
            SpeRegionPage::Lv1(WRITE_HTAB_SHADOW_WRITABLE_R3)
        } else {
            SpeRegionPage::Shadow(spe.shadow_real + (address - spe.shadow))
        });
    }
    None
}

#[cfg(test)]
#[path = "tests/shadow_tests.rs"]
mod tests;
