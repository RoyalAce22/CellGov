//! `write_htab_entry` for a page below the page shift of its MMIO
//! mapping, and for the pages on each side of the end of the first of
//! the 4.93 kernel's two MMIO mappings.

use super::*;
use crate::{page_table_storage, DeviceBusChange, MmioMapping};
use cellgov_event::UnitId;
use cellgov_ps3_abi::hw::ppu_mmu::{PTE_G, PTE_I};
use cellgov_ps3_abi::lv1::device_bus::{GAME_OS_MMIO_MAPPINGS, MMIO_REAL_BASE};
use cellgov_time::GuestTicks;

/// The vas id that the console capture `tests/micro/lv2_vas_boot_record`
/// reads from the 4.93 kernel's record.
const VAS_ID: u64 = 2;

/// An entry index, and a valid doubleword 0 with no large page bit.
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

/// The 4 KB entry with doubleword 1 `d1`, in the selected 4.93 address
/// space, with `mappings` the MMIO mappings.
fn write(d1: u64, mappings: &[MmioMapping]) -> HypercallDisposition {
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
    for mapping in mappings {
        device_bus.apply(DeviceBusChange::MmioRegionMapped(*mapping));
    }
    let storage = GuestMemory::from_regions(vec![page_table_storage()]).expect("storage");
    let request = HypercallRequest {
        unit: UnitId::new(1),
        number: WRITE_HTAB_ENTRY,
        inputs: [0, INDEX, D0, d1, 0, 0, 0, 0],
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

/// lv1 refuses only a page shift above the shift that the mapping's
/// region keeps [OFW-4.93 lv1 a:0x2298A4-0x2298B8].
#[test]
fn a_4_kb_page_in_a_mapping_of_page_shift_16_maps_the_devices_mmio() {
    let first = mapping(0, 16);
    for offset in [0, 0x1000, 0xF000] {
        let d1 = (first.lpar_address + offset) | PTE_I | PTE_G;
        assert_eq!(
            stored_rpn(&write(d1, &[first])),
            Some(MMIO_REAL_BASE + first.bus_address + offset),
            "{offset:#x}"
        );
    }
}

/// The second mapping starts at the first byte past the first one.
#[test]
fn the_pages_on_each_side_of_the_first_mappings_end_take_their_own_mappings() {
    let first = mapping(0, GAME_OS_MMIO_MAPPINGS[0].page_shift);
    let second = mapping(1, GAME_OS_MMIO_MAPPINGS[1].page_shift);
    assert_eq!(first.lpar_address + first.size, second.lpar_address);
    let both = [first, second];
    let last = first.lpar_address + first.size - 0x1000;
    assert_eq!(
        stored_rpn(&write(last | PTE_I | PTE_G, &both)),
        Some(MMIO_REAL_BASE + first.bus_address + 0xF000)
    );
    assert_eq!(
        stored_rpn(&write(second.lpar_address | PTE_I | PTE_G, &both)),
        Some(MMIO_REAL_BASE + second.bus_address)
    );
}
