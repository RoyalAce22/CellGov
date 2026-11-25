use super::*;
use crate::group::memory::storage::{region_page, AllocatedPage};

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

/// A region of one 64 KB page at counter value 4.
const THIRD: MemoryRegion = MemoryRegion {
    lpar_address: 0x0000_5000_0050_0000,
    size: 0x1_0000,
    page_size_exp: 16,
    unit_id: 1,
    counter: 4,
};

fn boot_regions() -> MemoryState {
    let mut memory = MemoryState::default();
    memory.create(FIRST);
    memory.create(SECOND);
    memory
}

/// Removes `region` and its storage from `memory`, as a release does.
fn release(memory: &mut MemoryState, region: &MemoryRegion) {
    memory.regions.retain(|r| r != region);
    memory.storage.remove(&region.counter);
}

#[test]
fn a_later_region_keeps_its_storage_after_an_earlier_region_leaves() {
    let mut memory = boot_regions();
    release(&mut memory, &FIRST);
    let real = ALLOCATED_STORAGE_BASE + FIRST.size;
    assert_eq!(memory.storage_of(&SECOND), Some(real));
    assert_eq!(
        region_page(&memory, SECOND.lpar_address + 0x1_0000, 16),
        Some(AllocatedPage::Stored(real + 0x1_0000))
    );
    assert_eq!(region_page(&memory, FIRST.lpar_address, 20), None);
}

#[test]
fn a_later_region_keeps_its_hash_terms_after_an_earlier_region_leaves() {
    let mut memory = boot_regions();
    release(&mut memory, &FIRST);
    let expected = lanes::value_term(REGION_COUNTER_SOURCE, 0, &4u64)
        .wrapping_add(lanes::value_term(REGION_SOURCE, 3, &SECOND))
        .wrapping_add(lanes::value_term(
            REGION_STORAGE_SOURCE,
            3,
            &(ALLOCATED_STORAGE_BASE + FIRST.size),
        ))
        .wrapping_add(lanes::value_term(
            STORAGE_END_SOURCE,
            0,
            &(ALLOCATED_STORAGE_BASE + FIRST.size + SECOND.size),
        ));
    assert_eq!(memory.sync_term(), expected);
}

#[test]
fn a_new_region_gets_storage_past_the_range_of_the_last_region_that_left() {
    let mut memory = MemoryState::default();
    memory.create(SECOND);
    release(&mut memory, &SECOND);
    memory.create(THIRD);
    assert_eq!(
        memory.storage_of(&THIRD),
        Some(ALLOCATED_STORAGE_BASE + SECOND.size)
    );
}

#[test]
fn a_new_region_gets_no_storage_when_only_a_released_range_has_room() {
    let mut memory = boot_regions();
    assert_eq!(
        FIRST.size + SECOND.size,
        storage::ALLOCATED_STORAGE_BYTES,
        "the boot regions fill the storage"
    );
    release(&mut memory, &SECOND);
    memory.create(THIRD);
    assert_eq!(memory.storage_of(&THIRD), None);
    assert_eq!(
        region_page(&memory, THIRD.lpar_address, 16),
        Some(AllocatedPage::NoStorage)
    );
}
