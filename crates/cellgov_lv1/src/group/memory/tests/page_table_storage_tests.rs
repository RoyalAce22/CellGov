use super::*;
use crate::group::memory::address_space::select;
use crate::group::memory::{ADDRESS_SPACE_SOURCE, PAGE_TABLE_STORAGE_SOURCE};
use crate::request::CallerTranslation;
use cellgov_event::UnitId;
use cellgov_mem::lanes;
use cellgov_ps3_abi::hw::ppu_mmu::PTE_L;
use cellgov_ps3_abi::lv1::address_space::game_os_boot_vas_id;
use cellgov_ps3_abi::lv1::call::SELECT_VIRTUAL_ADDRESS_SPACE;
use cellgov_ps3_abi::lv1::memory::{game_os_region_counter_at_boot, region_address};
use cellgov_ps3_abi::lv1::page_table::PTE_M;
use cellgov_time::GuestTicks;

/// The size of a page table of size exponent 18: 256 KB.
const TABLE_BYTES: u64 = 0x4_0000;

/// The number of 256 KB page tables that the storage holds.
const TABLES_IN_STORAGE: u64 = PAGE_TABLE_STORAGE_BYTES / TABLE_BYTES;

/// A valid doubleword 0 for a small page.
const VALID_DOUBLEWORD0: u64 = 0x0000_1000_0000_0001;

/// The vas id that the 4.93 hypervisor gives the kernel's address space.
fn first() -> u64 {
    game_os_boot_vas_id("4.93").expect("the 4.93 boot vas id")
}

/// The vas id of the address space with the construction index `index`.
fn vas_id(index: u64) -> u64 {
    first() + index
}

fn space(vas_id: u64) -> AddressSpace {
    AddressSpace {
        vas_id,
        htab_size_exp: 18,
        page_size_count: 2,
        page_shifts: 0x1014_0000_0000_0000,
        mapping: None,
    }
}

/// A state with `count` constructed address spaces of 256 KB tables.
fn spaces(count: u64) -> AddressSpaceState {
    let mut spaces = AddressSpaceState::default();
    for index in 0..count {
        spaces.apply(AddressSpaceChange::Constructed(space(vas_id(index))));
    }
    spaces
}

fn request(number: u64, inputs: [u64; 4]) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs: [inputs[0], inputs[1], inputs[2], inputs[3], 0, 0, 0, 0],
        pc: 0x8000_0000_0005_d948,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(9),
    }
}

fn storage() -> GuestMemory {
    GuestMemory::from_regions(vec![page_table_storage()]).expect("storage")
}

fn write(
    spaces: &AddressSpaceState,
    storage: &GuestMemory,
    inputs: [u64; 4],
) -> HypercallDisposition {
    write_entry(
        &request(WRITE_HTAB_ENTRY, inputs),
        Some("4.93"),
        spaces,
        &LogicalSpeState::default(),
        &MemoryState::default(),
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

/// The range of the one write that `d` carries.
fn written_range(d: &HypercallDisposition) -> ByteRange {
    let HypercallDisposition::Answered(answer) = d else {
        panic!("{d:?}");
    };
    assert_eq!(answer.effects.len(), 1, "{:?}", answer.effects);
    match &answer.effects[0] {
        Effect::SharedWriteIntent { range, .. } => *range,
        other => panic!("{other:?}"),
    }
}

#[test]
fn each_address_space_takes_its_own_page_table_storage() {
    let spaces = spaces(2);
    assert_eq!(spaces.table_real(vas_id(0)), Some(PAGE_TABLE_STORAGE_BASE));
    assert_eq!(
        spaces.table_real(vas_id(1)),
        Some(PAGE_TABLE_STORAGE_BASE + TABLE_BYTES)
    );
}

#[test]
fn a_page_table_starts_at_the_next_multiple_of_its_size() {
    assert_eq!(
        placement(PAGE_TABLE_STORAGE_BASE, 18),
        Some(PAGE_TABLE_STORAGE_BASE)
    );
    assert_eq!(
        placement(PAGE_TABLE_STORAGE_BASE + TABLE_BYTES, 19),
        Some(PAGE_TABLE_STORAGE_BASE + 2 * TABLE_BYTES)
    );
    assert_eq!(placement(PAGE_TABLE_STORAGE_BASE + TABLE_BYTES, 20), None);
}

#[test]
fn a_page_table_that_the_storage_has_no_room_for_gets_no_storage() {
    let spaces = spaces(TABLES_IN_STORAGE + 1);
    let last = PAGE_TABLE_STORAGE_BASE + (TABLES_IN_STORAGE - 1) * TABLE_BYTES;
    assert_eq!(spaces.table_real(vas_id(TABLES_IN_STORAGE - 1)), Some(last));
    assert_eq!(spaces.table_real(vas_id(TABLES_IN_STORAGE)), None);
}

#[test]
fn a_selection_loads_sdr1_with_the_selected_address_space_s_table() {
    let spaces = spaces(2);
    let d = select(
        &request(SELECT_VIRTUAL_ADDRESS_SPACE, [vas_id(1), 0, 0, 0]),
        CallerTranslation::Loads,
        Some("4.93"),
        &spaces,
        &storage(),
    );
    let HypercallDisposition::Answered(answer) = d else {
        panic!("{d:?}");
    };
    assert_eq!(
        answer.page_table.map(|load| load.sdr1),
        Some(select_sdr1(PAGE_TABLE_STORAGE_BASE + TABLE_BYTES, 18))
    );
}

#[test]
fn a_selection_of_an_address_space_whose_table_has_no_storage_refuses() {
    let spaces = spaces(TABLES_IN_STORAGE + 1);
    let d = select(
        &request(
            SELECT_VIRTUAL_ADDRESS_SPACE,
            [vas_id(TABLES_IN_STORAGE), 0, 0, 0],
        ),
        CallerTranslation::Loads,
        Some("4.93"),
        &spaces,
        &storage(),
    );
    assert_eq!(
        refusal_what(&d),
        Some("a selection of an address space with no storage for its page table")
    );
}

#[test]
fn a_write_on_the_second_address_space_lands_in_its_own_table() {
    let d = write(&spaces(2), &storage(), [vas_id(1), 3, 0, 0]);
    assert_eq!(
        written_range(&d).start().raw(),
        PAGE_TABLE_STORAGE_BASE + TABLE_BYTES + 3 * PTE_BYTES as u64
    );
}

#[test]
fn the_duplicate_search_reads_only_the_named_address_space_s_table() {
    let mut storage = storage();
    let mut bytes = VALID_DOUBLEWORD0.to_be_bytes().to_vec();
    bytes.extend_from_slice(&0u64.to_be_bytes());
    let range = ByteRange::new(GuestAddr::new(PAGE_TABLE_STORAGE_BASE), 16).expect("range");
    storage.apply_commit(range, &bytes).expect("commit");
    let spaces = spaces(2);
    let page = 0x0040_0000;
    let on_first = write(&spaces, &storage, [vas_id(0), 1, VALID_DOUBLEWORD0, page]);
    assert_eq!(on_first, lv1_answer(WRITE_HTAB_DUPLICATE_R3));
    let on_second = write(&spaces, &storage, [vas_id(1), 1, VALID_DOUBLEWORD0, page]);
    assert_eq!(
        refusal_what(&on_second),
        Some("a valid entry whose page CellGov holds no storage for")
    );
}

#[test]
fn a_read_on_the_second_address_space_reads_its_own_table() {
    let mut storage = storage();
    let at = PAGE_TABLE_STORAGE_BASE + TABLE_BYTES;
    let range = ByteRange::new(GuestAddr::new(at), 8).expect("range");
    storage
        .apply_commit(range, &VALID_DOUBLEWORD0.to_be_bytes())
        .expect("commit");
    let d = read_entries(
        &request(READ_HTAB_ENTRIES, [vas_id(1), 0, 0, 0]),
        Some("4.93"),
        &spaces(2),
        &storage,
    );
    let HypercallDisposition::Answered(answer) = d else {
        panic!("{d:?}");
    };
    assert_eq!(answer.outputs().expect("r4 to r10")[0], VALID_DOUBLEWORD0);
}

/// The 4.93 address of a 256 KB page table mapping at the first counter value.
fn mapping_address() -> u64 {
    let counter = game_os_region_counter_at_boot("4.93").expect("4.93 boot counter");
    region_address(TABLE_BYTES, counter).expect("address")
}

#[test]
fn a_page_in_the_second_table_s_mapping_lies_in_the_second_table() {
    let mut spaces = spaces(2);
    let lpar_address = mapping_address();
    spaces.apply(AddressSpaceChange::Mapped {
        vas_id: vas_id(1),
        lpar_address,
        counter: 0,
    });
    let d0 = 0x0000_0012_3456_7801 | PTE_L;
    let d = write(
        &spaces,
        &storage(),
        [vas_id(0), 8, d0, (lpar_address + 0x1_0000) | 0x2],
    );
    let HypercallDisposition::Answered(answer) = &d else {
        panic!("{d:?}");
    };
    let Effect::SharedWriteIntent { bytes, .. } = &answer.effects[0] else {
        panic!("{:?}", answer.effects);
    };
    let real = PAGE_TABLE_STORAGE_BASE + TABLE_BYTES + 0x1_0000;
    assert_eq!(bytes.bytes()[8..16], (real | PTE_M | 0x2).to_be_bytes());
}

#[test]
fn a_page_in_the_mapping_of_a_table_with_no_storage_refuses() {
    let mut spaces = spaces(TABLES_IN_STORAGE + 1);
    let lpar_address = mapping_address();
    spaces.apply(AddressSpaceChange::Mapped {
        vas_id: vas_id(TABLES_IN_STORAGE),
        lpar_address,
        counter: 0,
    });
    let d0 = 0x0000_0012_3456_7801 | PTE_L;
    let d = write(
        &spaces,
        &storage(),
        [vas_id(0), 8, d0, (lpar_address + 0x1_0000) | 0x2],
    );
    assert_eq!(
        refusal_what(&d),
        Some("a valid entry whose page lies in the mapping of a page table with no storage")
    );
}

#[test]
fn each_page_table_with_storage_adds_its_real_address_under_its_construction_index() {
    let spaces = spaces(TABLES_IN_STORAGE + 1);
    let mut expected = 0u128;
    for index in 0..=TABLES_IN_STORAGE {
        let term = lanes::value_term(ADDRESS_SPACE_SOURCE, index, &space(vas_id(index)));
        expected = expected.wrapping_add(term);
    }
    for index in 0..TABLES_IN_STORAGE {
        let table = PAGE_TABLE_STORAGE_BASE + index * TABLE_BYTES;
        let term = lanes::value_term(PAGE_TABLE_STORAGE_SOURCE, index, &table);
        expected = expected.wrapping_add(term);
    }
    assert_eq!(spaces.sync_term(), expected);
}
