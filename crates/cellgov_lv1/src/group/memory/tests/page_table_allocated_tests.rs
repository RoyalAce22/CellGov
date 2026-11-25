//! `write_htab_entry` for a page in a region that `allocate_memory`
//! created.

use super::*;
use crate::{allocated_storage, AddressSpaceChange, MemoryRegion, ALLOCATED_STORAGE_BASE};
use cellgov_event::UnitId;
use cellgov_ps3_abi::hw::ppu_mmu::{PTE_L, PTE_N};
use cellgov_ps3_abi::lv1::call::WRITE_HTAB_ENTRY;
use cellgov_ps3_abi::lv1::memory::WRITE_HTAB_ALLOCATED_PAGE_SIZE_R3;
use cellgov_ps3_abi::lv1::page_table::PTE_M;
use cellgov_time::GuestTicks;

/// The vas id that the console capture `tests/micro/lv2_vas_boot_record`
/// reads from the 4.93 kernel's record.
const VAS_ID: u64 = 2;

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

/// The entry index, doubleword 0 and doubleword 1 of the first call that
/// CellGov's boot of the 4.93 kernel makes for a page of its second
/// region [OFW-4.93 lv2_kernel a:0x800000000005e4f0]. Doubleword 1 holds
/// the page at offset 0x80000 of the region, N and M, with PP = 0b00.
const KERNEL_INDEX: u64 = 0x1;
const KERNEL_D0: u64 = 0x1041;
const KERNEL_D1: u64 = 0x0000_5000_0038_0000 | PTE_M | PTE_N;

fn request(index: u64, d0: u64, d1: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number: WRITE_HTAB_ENTRY,
        inputs: [0, index, d0, d1, 0, 0, 0, 0],
        pc: 0x8000_0000_0005_e4f0,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(63_971),
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

fn boot_regions() -> MemoryState {
    let mut memory = MemoryState::default();
    memory.create(FIRST);
    memory.create(SECOND);
    memory
}

fn storage() -> GuestMemory {
    GuestMemory::from_regions(vec![page_table_storage(), allocated_storage()]).expect("storage")
}

fn write(d0: u64, d1: u64, storage: &GuestMemory) -> HypercallDisposition {
    write_entry(
        &request(KERNEL_INDEX, d0, d1),
        Some("4.93"),
        &spaces(),
        &LogicalSpeState::default(),
        &boot_regions(),
        &DeviceBusState::default(),
        storage,
    )
}

fn refusal_what(d: &HypercallDisposition) -> Option<&'static str> {
    match d {
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput { what, .. }) => Some(what),
        _ => None,
    }
}

#[test]
fn the_kernels_entry_in_its_second_boot_region_maps_the_regions_storage() {
    let mut bytes = KERNEL_D0.to_be_bytes().to_vec();
    let stored = (ALLOCATED_STORAGE_BASE + 0x8_0000) | PTE_M | PTE_N;
    bytes.extend_from_slice(&stored.to_be_bytes());
    let mut answer = HypercallAnswer::lv1_status(0, [0; 7]);
    answer.effects.push(Effect::SharedWriteIntent {
        range: ByteRange::new(
            GuestAddr::new(entry_address(PAGE_TABLE_STORAGE_BASE, KERNEL_INDEX)),
            16,
        )
        .expect("range"),
        bytes: WritePayload::new(bytes),
        source: UnitId::new(1),
        source_time: GuestTicks::new(63_971),
    });
    assert_eq!(
        write(KERNEL_D0, KERNEL_D1, &storage()),
        HypercallDisposition::Answered(answer)
    );
}

/// The 4.93 address space's large page index 1 is the 1 MB page; see
/// [`large_page_bits`].
#[test]
fn a_1_mb_page_in_the_64_kb_page_region_gets_lv1s_page_size_r3() {
    let d1 = SECOND.lpar_address | large_page_bits(1) | PTE_M;
    assert_eq!(
        write(KERNEL_D0 | PTE_L, d1, &storage()),
        lv1_answer(WRITE_HTAB_ALLOCATED_PAGE_SIZE_R3)
    );
}

#[test]
fn a_page_of_the_first_boot_region_is_refused_for_want_of_storage() {
    let d = write(KERNEL_D0, FIRST.lpar_address | PTE_M, &storage());
    assert_eq!(
        refusal_what(&d),
        Some("a valid entry whose page lies in an allocated memory region with no storage")
    );
}

/// lv1's writer searches for a duplicate before it translates the page
/// [OFW-4.93 lv1 a:0x2D2F20-0x2D3038], so the missing storage does not
/// decide the answer.
#[test]
fn a_duplicate_entry_for_a_page_with_no_storage_answers_duplicate_entry() {
    let mut storage = storage();
    let other = (KERNEL_INDEX & !7) + 2;
    let range = ByteRange::new(
        GuestAddr::new(entry_address(PAGE_TABLE_STORAGE_BASE, other)),
        8,
    )
    .expect("range");
    storage
        .apply_commit(range, &KERNEL_D0.to_be_bytes())
        .expect("commit");
    assert_eq!(
        write(KERNEL_D0, FIRST.lpar_address | PTE_M, &storage),
        lv1_answer(WRITE_HTAB_DUPLICATE_R3)
    );
}

#[test]
fn a_page_of_the_second_boot_region_with_no_storage_region_is_refused() {
    let table_only = GuestMemory::from_regions(vec![page_table_storage()]).expect("storage");
    assert_eq!(
        refusal_what(&write(KERNEL_D0, KERNEL_D1, &table_only)),
        Some("a valid entry whose page CellGov holds no storage for")
    );
}
