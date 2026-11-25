//! `write_htab_entry` for a page in an MMIO mapping that
//! `map_device_mmio_region` made.

use super::*;
use crate::{
    allocated_storage, page_table_storage, AddressSpaceChange, DeviceBusChange, MemoryRegion,
    MmioMapping,
};
use cellgov_event::UnitId;
use cellgov_ps3_abi::hw::ppu_mmu::{PTE_G, PTE_I, PTE_L, PTE_N};
use cellgov_ps3_abi::lv1::call::WRITE_HTAB_ENTRY;
use cellgov_ps3_abi::lv1::device_bus::{
    GAME_OS_MMIO_MAPPINGS, MMIO_REAL_BASE, WRITE_HTAB_MMIO_PAGE_SIZE_R3,
};
use cellgov_time::GuestTicks;

/// The vas id that the console capture `tests/micro/lv2_vas_boot_record`
/// reads from the 4.93 kernel's record.
const VAS_ID: u64 = 2;

/// The mapping that the console's lv1 made for the 4.93 kernel's call:
/// the region at bus address 0x3010000 of device 1 of bus 1, 64 KB, at
/// the logical partition address of the capture
/// `tests/micro/lv1_device_mmio_map`.
const MAPPING: MmioMapping = MmioMapping {
    bus: 1,
    device: 1,
    bus_address: 0x301_0000,
    size: 0x1_0000,
    lpar_address: 0x4000_0007_0000,
    page_shift: 12,
};

/// The entry index, doubleword 0 and doubleword 1 of the call that
/// CellGov's boot of the 4.93 kernel makes for the first page of the
/// mapping [OFW-4.93 lv2_kernel a:0x800000000005e4f0]. Doubleword 1 holds
/// the page, I, G and N, with PP = 0b00.
const KERNEL_INDEX: u64 = 0x89;
const KERNEL_D0: u64 = 0x1_0001;
const KERNEL_D1: u64 = 0x0000_4000_0007_002C;

fn request(d0: u64, d1: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number: WRITE_HTAB_ENTRY,
        inputs: [0, KERNEL_INDEX, d0, d1, 0, 0, 0, 0],
        pc: 0x8000_0000_0005_e4f0,
        lr: 0,
        thread: Some(1),
        problem_state: false,
        tick: GuestTicks::new(36_822),
    }
}

/// The 4.93 kernel's address space, selected: a 256 KB page table with
/// page shifts 16 and 20.
fn spaces() -> AddressSpaceState {
    let mut spaces = AddressSpaceState::default();
    spaces.apply(AddressSpaceChange::Constructed(AddressSpace {
        vas_id: VAS_ID,
        htab_size_exp: 18,
        page_size_count: 2,
        page_shifts: 0x1014_0000_0000_0000,
        mapping: None,
    }));
    spaces.apply(AddressSpaceChange::Selected(VAS_ID));
    spaces
}

fn mapped(mapping: MmioMapping) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    state.apply(DeviceBusChange::MmioRegionMapped(mapping));
    state
}

fn write(d0: u64, d1: u64, device_bus: &DeviceBusState) -> HypercallDisposition {
    let storage = GuestMemory::from_regions(vec![page_table_storage()]).expect("storage");
    write_entry(
        &request(d0, d1),
        Some("4.93"),
        &spaces(),
        &LogicalSpeState::default(),
        &MemoryState::default(),
        device_bus,
        &storage,
    )
}

fn refusal_what(d: &HypercallDisposition) -> Option<&'static str> {
    match d {
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput { what, .. }) => Some(what),
        _ => None,
    }
}

/// lv1 translates the page to the region's real address, 0x24003010000,
/// and stores I and G for that address in place of the caller's WIMG.
#[test]
fn the_kernels_entry_in_its_mmio_mapping_maps_the_devices_real_address() {
    assert_eq!(GAME_OS_MMIO_MAPPINGS[0].lpar_address, MAPPING.lpar_address);
    let mut bytes = KERNEL_D0.to_be_bytes().to_vec();
    let stored = (MMIO_REAL_BASE + 0x301_0000) | PTE_I | PTE_G | PTE_N;
    assert_eq!(stored, 0x0000_0240_0301_002C);
    bytes.extend_from_slice(&stored.to_be_bytes());
    let mut answer = HypercallAnswer::lv1_status(0, [0; 7]);
    answer.effects.push(Effect::SharedWriteIntent {
        range: ByteRange::new(GuestAddr::new(entry_address(KERNEL_INDEX)), 16).expect("range"),
        bytes: WritePayload::new(bytes),
        source: UnitId::new(1),
        source_time: GuestTicks::new(36_822),
    });
    assert_eq!(
        write(KERNEL_D0, KERNEL_D1, &mapped(MAPPING)),
        HypercallDisposition::Answered(answer)
    );
}

/// The page at offset 0x1000 of the region lies at offset 0x1000 of the
/// device's MMIO.
#[test]
fn a_page_past_the_first_maps_the_same_offset_of_the_devices_mmio() {
    let d = write(KERNEL_D0, KERNEL_D1 + 0x1000, &mapped(MAPPING));
    let HypercallDisposition::Answered(answer) = d else {
        panic!("not answered: {d:?}");
    };
    let [Effect::SharedWriteIntent { bytes, .. }] = answer.effects.as_slice() else {
        panic!("not one write: {:?}", answer.effects);
    };
    let stored = read_u64(bytes.bytes(), 8);
    assert_eq!(stored & PTE_RPN, MMIO_REAL_BASE + 0x301_1000);
}

/// The 4.93 address space's page size index 0 is the 64 KB page, above
/// the mapping's 4 KB page shift; see [`large_page_bits`].
#[test]
fn a_64_kb_page_in_the_4_kb_page_mapping_gets_lv1s_page_size_r3() {
    let d1 = MAPPING.lpar_address | large_page_bits(0) | PTE_I | PTE_G;
    assert_eq!(
        write(KERNEL_D0 | PTE_L, d1, &mapped(MAPPING)),
        lv1_answer(WRITE_HTAB_MMIO_PAGE_SIZE_R3)
    );
}

/// A 64 KB allocation at region counter value 7 takes the mapping's
/// address.
#[test]
fn a_page_in_an_mmio_mapping_and_an_allocated_region_is_refused() {
    assert_eq!(region_address(0x1_0000, 7), Some(MAPPING.lpar_address));
    let mut allocated = MemoryState::default();
    allocated.create(MemoryRegion {
        lpar_address: MAPPING.lpar_address,
        size: 0x1_0000,
        page_size_exp: 16,
        unit_id: 1,
        counter: 7,
    });
    let storage = GuestMemory::from_regions(vec![page_table_storage(), allocated_storage()])
        .expect("storage");
    let d = write_entry(
        &request(KERNEL_D0, KERNEL_D1),
        Some("4.93"),
        &spaces(),
        &LogicalSpeState::default(),
        &allocated,
        &mapped(MAPPING),
        &storage,
    );
    assert_eq!(
        refusal_what(&d),
        Some("a valid entry whose page lies in an MMIO mapping and in another region")
    );
}

#[test]
fn the_page_with_no_mmio_mapping_is_refused_outside_the_real_mode_region() {
    assert_eq!(
        refusal_what(&write(KERNEL_D0, KERNEL_D1, &DeviceBusState::default())),
        Some("a valid entry whose page lies outside the partition's real mode region")
    );
}
