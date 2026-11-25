//! `write_htab_entry` for a page in each of the 4.93 kernel's MMIO
//! mappings, with the page shift that the mapping keeps.

use super::*;
use crate::{page_table_storage, DeviceBusChange, MmioMapping};
use cellgov_event::UnitId;
use cellgov_ps3_abi::hw::ppu_mmu::{PTE_G, PTE_I, PTE_L, PTE_N};
use cellgov_ps3_abi::lv1::device_bus::{
    GAME_OS_MMIO_MAPPINGS, MMIO_REAL_BASE, WRITE_HTAB_MMIO_PAGE_SIZE_R3,
};
use cellgov_time::GuestTicks;

/// The vas id that the console capture `tests/micro/lv2_vas_boot_record`
/// reads from the 4.93 kernel's record.
const VAS_ID: u64 = 2;

/// The entry index and doubleword 0 of the kernel's entry for the first
/// page of an MMIO mapping [OFW-4.93 lv2_kernel a:0x800000000005e4f0].
const INDEX: u64 = 0x89;
const D0: u64 = 0x1_0001;

/// The mapping of the console's record `row` of the kernel's calls, with
/// the page shift `page_shift`.
fn mapping(row: usize, page_shift: u64) -> MmioMapping {
    let row = GAME_OS_MMIO_MAPPINGS[row];
    MmioMapping {
        bus: row.bus,
        device: row.device,
        bus_address: row.bus_address,
        size: row.size,
        lpar_address: row.lpar_address,
        page_shift,
    }
}

/// The entry with doublewords `d0` and `d1`, in the selected 4.93 address
/// space, with `mapping` the one MMIO mapping.
fn write(d0: u64, d1: u64, mapping: MmioMapping) -> HypercallDisposition {
    let mut spaces = AddressSpaceState::default();
    spaces.apply(AddressSpaceChange::Constructed(AddressSpace {
        vas_id: VAS_ID,
        htab_size_exp: 18,
        page_size_count: 2,
        page_shifts: 0x1014_0000_0000_0000,
        mapping: None,
    }));
    spaces.apply(AddressSpaceChange::Selected(VAS_ID));
    let mut device_bus = DeviceBusState::default();
    device_bus.apply(DeviceBusChange::MmioRegionMapped(mapping));
    let storage = GuestMemory::from_regions(vec![page_table_storage()]).expect("storage");
    let request = HypercallRequest {
        unit: UnitId::new(1),
        number: WRITE_HTAB_ENTRY,
        inputs: [0, INDEX, d0, d1, 0, 0, 0, 0],
        pc: 0x8000_0000_0005_e4f0,
        lr: 0,
        thread: Some(1),
        problem_state: false,
        tick: GuestTicks::new(37_514),
    };
    write_entry(
        &request,
        Some("4.93"),
        &spaces,
        &LogicalSpeState::default(),
        &MemoryState::default(),
        &device_bus,
        &storage,
    )
}

/// The real page number that the answer stores, or `None` when it stores
/// no entry.
fn stored_rpn(d: &HypercallDisposition) -> Option<u64> {
    let HypercallDisposition::Answered(answer) = d else {
        return None;
    };
    match answer.effects.as_slice() {
        [Effect::SharedWriteIntent { bytes, .. }] => Some(read_u64(bytes.bytes(), 8) & PTE_RPN),
        _ => None,
    }
}

#[test]
fn the_second_kernel_mapping_maps_its_pages_to_the_third_regions_real_address() {
    let second = mapping(1, GAME_OS_MMIO_MAPPINGS[1].page_shift);
    assert_eq!(second.bus_address, 0x381_0000);
    for offset in [0, 0x1000, 0xF000] {
        let d1 = (second.lpar_address + offset) | PTE_I | PTE_G | PTE_N;
        assert_eq!(
            stored_rpn(&write(D0, d1, second)),
            Some(MMIO_REAL_BASE + 0x381_0000 + offset),
            "{offset:#x}"
        );
    }
}

/// The 4.93 address space's page size index 0 is the 64 KB page; see
/// [`large_page_bits`].
#[test]
fn a_64_kb_page_passes_in_a_mapping_of_page_shift_16_and_fails_in_one_of_12() {
    let first = mapping(0, 16);
    let d1 = first.lpar_address | large_page_bits(0) | PTE_I | PTE_G;
    assert_eq!(
        stored_rpn(&write(D0 | PTE_L, d1, first)),
        Some(MMIO_REAL_BASE + first.bus_address)
    );
    assert_eq!(
        write(D0 | PTE_L, d1, mapping(0, 12)),
        lv1_answer(WRITE_HTAB_MMIO_PAGE_SIZE_R3)
    );
}
