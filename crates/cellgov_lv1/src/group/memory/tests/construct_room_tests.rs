use super::*;
use cellgov_event::UnitId;
use cellgov_time::GuestTicks;

/// The size of a page table of size exponent 18: 256 KB.
const TABLE_BYTES: u64 = 0x4_0000;

/// The number of 256 KB page tables that the storage holds.
const TABLES_IN_STORAGE: u64 = page_table::PAGE_TABLE_STORAGE_BYTES / TABLE_BYTES;

const NO_ROOM_REFUSAL: &str =
    "an address space construction whose page table the page table storage has no room for";
const SECOND_CONSTRUCTION_REFUSAL: &str =
    "a second address space construction, whose vas id no source gives";

/// The call the 4.93 kernel makes
/// [OFW-4.93 lv2_kernel a:0x800000000005dcc0-0x800000000005dcf4]: r3 = 0,
/// two page sizes, shifts 16 and 20.
fn kernel_call() -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[1] = KERNEL_PAGE_SIZE_COUNT;
    inputs[2] = KERNEL_PAGE_SHIFTS;
    HypercallRequest {
        unit: UnitId::new(1),
        number: CONSTRUCT_VIRTUAL_ADDRESS_SPACE,
        inputs,
        pc: 0x8000_0000_0005_dcf4,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(9),
    }
}

/// A state with `count` constructed address spaces of 256 KB tables.
fn spaces(count: u64) -> AddressSpaceState {
    let mut spaces = AddressSpaceState::default();
    for vas_id in 2..2 + count {
        spaces.apply(AddressSpaceChange::Constructed(AddressSpace {
            vas_id,
            htab_size_exp: HTAB_SIZE_EXP_MIN,
            page_size_count: KERNEL_PAGE_SIZE_COUNT,
            page_shifts: KERNEL_PAGE_SHIFTS,
            mapping: None,
        }));
    }
    spaces
}

fn refusal_what(d: &HypercallDisposition) -> Option<&'static str> {
    match d {
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput { what, .. }) => Some(what),
        _ => None,
    }
}

#[test]
fn the_next_table_follows_the_tables_with_storage_until_the_storage_is_full() {
    let last = page_table::PAGE_TABLE_STORAGE_BASE + (TABLES_IN_STORAGE - 1) * TABLE_BYTES;
    assert_eq!(
        spaces(TABLES_IN_STORAGE - 1).next_table(HTAB_SIZE_EXP_MIN),
        Some(last)
    );
    assert_eq!(
        spaces(TABLES_IN_STORAGE).next_table(HTAB_SIZE_EXP_MIN),
        None
    );
}

#[test]
fn a_construction_whose_page_table_has_no_room_refuses() {
    let d = construct(&kernel_call(), Some("4.93"), &spaces(TABLES_IN_STORAGE));
    assert_eq!(refusal_what(&d), Some(NO_ROOM_REFUSAL));
}

#[test]
fn a_construction_whose_page_table_has_room_passes_the_room_check() {
    let d = construct(&kernel_call(), Some("4.93"), &spaces(TABLES_IN_STORAGE - 1));
    assert_eq!(refusal_what(&d), Some(SECOND_CONSTRUCTION_REFUSAL));
}
