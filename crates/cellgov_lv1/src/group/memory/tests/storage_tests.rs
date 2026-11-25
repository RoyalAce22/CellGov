use super::*;
use crate::{page_table_storage, shadow_storage};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};

/// The kernel's first boot region in the console capture
/// `lv2_repository_boot_nodes`: 0x7900000 bytes in 1 MB pages at counter
/// value 2.
const FIRST: MemoryRegion = MemoryRegion {
    lpar_address: 0x0000_6C00_1000_0000,
    size: 0x790_0000,
    page_size_exp: 20,
    unit_id: 1,
    counter: 2,
};

/// The kernel's second boot region in the same capture: 0xA0000 bytes in
/// 64 KB pages at counter value 3.
const SECOND: MemoryRegion = MemoryRegion {
    lpar_address: 0x0000_5000_0030_0000,
    size: 0xA_0000,
    page_size_exp: 16,
    unit_id: 1,
    counter: 3,
};

fn region(lpar_address: u64, size: u64, page_size_exp: u8) -> MemoryRegion {
    MemoryRegion {
        lpar_address,
        size,
        page_size_exp,
        ..SECOND
    }
}

fn boot_regions() -> MemoryState {
    let mut memory = MemoryState::default();
    memory.create(FIRST);
    memory.create(SECOND);
    memory
}

/// A region of one 1 MB page after the two boot regions, which fill the
/// storage.
const PAST_STORAGE: MemoryRegion = MemoryRegion {
    lpar_address: 0x0000_6400_0200_0000,
    size: 0x10_0000,
    page_size_exp: 20,
    unit_id: 1,
    counter: 4,
};

fn boot_regions_and_one_past_the_storage() -> MemoryState {
    let mut memory = boot_regions();
    memory.create(PAST_STORAGE);
    memory
}

#[test]
fn each_region_starts_at_its_page_size_past_the_last_stored_region() {
    let mut memory = MemoryState::default();
    memory.create(region(0x0000_4000_0010_0000, 0x1_0000, 16));
    memory.create(region(0x0000_5000_0020_0000, 0x10_0000, 20));
    memory.create(region(0x0000_4000_0030_0000, 0x1_0000, 16));
    assert_eq!(memory.storage_real(0), Some(ALLOCATED_STORAGE_BASE));
    assert_eq!(
        memory.storage_real(1),
        Some(ALLOCATED_STORAGE_BASE + 0x10_0000)
    );
    assert_eq!(
        memory.storage_real(2),
        Some(ALLOCATED_STORAGE_BASE + 0x20_0000)
    );
}

#[test]
fn a_region_after_one_with_no_storage_starts_past_the_last_stored_region() {
    let mut memory = MemoryState::default();
    memory.create(region(0x0000_4000_0010_0000, 0x1_0000, 16));
    memory.create(region(
        0x0000_6400_0200_0000,
        2 * ALLOCATED_STORAGE_BYTES,
        20,
    ));
    memory.create(region(0x0000_4000_0030_0000, 0x1_0000, 16));
    assert_eq!(memory.storage_real(0), Some(ALLOCATED_STORAGE_BASE));
    assert_eq!(memory.storage_real(1), None);
    assert_eq!(
        memory.storage_real(2),
        Some(ALLOCATED_STORAGE_BASE + 0x1_0000)
    );
}

#[test]
fn a_region_that_ends_at_the_storage_end_fits_and_a_larger_one_does_not() {
    let whole = region(0, ALLOCATED_STORAGE_BYTES, 16);
    assert_eq!(
        placement(ALLOCATED_STORAGE_BASE, &whole),
        Some(ALLOCATED_STORAGE_BASE)
    );
    let larger = region(0, ALLOCATED_STORAGE_BYTES + 0x1_0000, 16);
    assert_eq!(placement(ALLOCATED_STORAGE_BASE, &larger), None);
    let late = region(0, 0x1_0000, 16);
    let end = ALLOCATED_STORAGE_BASE + ALLOCATED_STORAGE_BYTES;
    assert_eq!(placement(end - 0x1_0000, &late), Some(end - 0x1_0000));
    assert_eq!(placement(end - 0x1_0000 + 1, &late), None);
}

#[test]
fn a_page_of_a_stored_region_lies_at_its_offset_in_the_storage() {
    let memory = boot_regions();
    let page = SECOND.lpar_address + 0x8_0000;
    let real = ALLOCATED_STORAGE_BASE + FIRST.size + 0x8_0000;
    assert_eq!(
        region_page(&memory, page, 12),
        Some(AllocatedPage::Stored(real))
    );
    assert_eq!(
        region_page(&memory, page, 16),
        Some(AllocatedPage::Stored(real))
    );
}

#[test]
fn a_page_larger_than_its_regions_pages_gets_lv1s_r3() {
    let memory = boot_regions();
    assert_eq!(
        region_page(&memory, SECOND.lpar_address, 20),
        Some(AllocatedPage::Lv1(WRITE_HTAB_ALLOCATED_PAGE_SIZE_R3))
    );
}

#[test]
fn a_page_of_a_region_with_no_storage_is_no_storage_and_one_past_every_region_is_none() {
    let memory = boot_regions_and_one_past_the_storage();
    assert_eq!(memory.storage_real(2), None);
    assert_eq!(
        region_page(&memory, PAST_STORAGE.lpar_address + 0x8_0000, 20),
        Some(AllocatedPage::NoStorage)
    );
    assert_eq!(
        region_page(&memory, SECOND.lpar_address + SECOND.size, 12),
        None
    );
    assert_eq!(region_page(&memory, SECOND.lpar_address - 0x1000, 12), None);
}

#[test]
fn the_allocated_storage_lies_above_the_other_storage_and_reads_zero() {
    let storage = GuestMemory::from_regions(vec![
        page_table_storage(),
        shadow_storage(),
        allocated_storage(),
    ])
    .expect("the storage regions do not overlap");
    for other in [page_table_storage(), shadow_storage()] {
        assert!(
            other.base() + other.size() <= ALLOCATED_STORAGE_BASE,
            "the storage at {:#x} of {:#x} bytes ends above the allocated storage",
            other.base(),
            other.size()
        );
    }
    let range = ByteRange::new(
        GuestAddr::new(ALLOCATED_STORAGE_BASE),
        ALLOCATED_STORAGE_BYTES,
    )
    .expect("range");
    let bytes = storage.read(range).expect("the storage holds the range");
    assert_eq!(bytes.len() as u64, ALLOCATED_STORAGE_BYTES);
    assert!(bytes.iter().all(|&b| b == 0));
}
