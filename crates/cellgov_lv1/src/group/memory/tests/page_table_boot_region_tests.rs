//! `write_htab_entry` for a page of the 4.93 kernel's first boot region.

use super::allocated_tests::{storage, stored_answer, write, FIRST, KERNEL_D0};
use crate::ALLOCATED_STORAGE_BASE;
use cellgov_ps3_abi::hw::ppu_mmu::PTE_N;
use cellgov_ps3_abi::lv1::page_table::PTE_M;

#[test]
fn an_entry_for_the_first_page_of_the_first_boot_region_maps_the_storage_base() {
    let d1 = FIRST.lpar_address | PTE_M | PTE_N;
    assert_eq!(
        write(KERNEL_D0, d1, &storage()),
        stored_answer(ALLOCATED_STORAGE_BASE | PTE_M | PTE_N)
    );
}

#[test]
fn an_entry_for_the_last_page_of_the_first_boot_region_maps_its_offset_in_the_storage() {
    let offset = FIRST.size - 0x1000;
    let d1 = (FIRST.lpar_address + offset) | PTE_M | PTE_N;
    assert_eq!(
        write(KERNEL_D0, d1, &storage()),
        stored_answer((ALLOCATED_STORAGE_BASE + offset) | PTE_M | PTE_N)
    );
}
