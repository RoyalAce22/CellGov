use super::*;
use crate::answer::PageTableLoad;
use crate::page_table::{page_table_storage, PAGE_TABLE_STORAGE_BASE};
use cellgov_event::UnitId;
use cellgov_mem::{PageSize, Region};
use cellgov_ps3_abi::lv1::call::SELECT_VIRTUAL_ADDRESS_SPACE;
use cellgov_time::GuestTicks;

/// The vas id that the console capture `tests/micro/lv2_vas_boot_record`
/// reads from the 4.93 kernel's record.
const CAPTURED_VAS_ID: u64 = 2;

fn select_request() -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[0] = CAPTURED_VAS_ID;
    HypercallRequest {
        unit: UnitId::new(1),
        number: SELECT_VIRTUAL_ADDRESS_SPACE,
        inputs,
        pc: 0x8000_0000_0005_dd10,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(9),
    }
}

/// The address space the 4.93 kernel constructs: a 256 KB page table
/// and page shifts 16 and 20.
fn constructed() -> AddressSpaceState {
    let mut spaces = AddressSpaceState::default();
    spaces.apply(AddressSpaceChange::Constructed(AddressSpace {
        vas_id: CAPTURED_VAS_ID,
        htab_size_exp: 18,
        page_size_count: 2,
        page_shifts: 0x1014_0000_0000_0000,
        mapping: None,
    }));
    spaces
}

fn load_of(d: &HypercallDisposition) -> Option<PageTableLoad> {
    match d {
        HypercallDisposition::Answered(answer) => answer.page_table,
        HypercallDisposition::Refused(_) => None,
    }
}

#[test]
fn a_selection_loads_sdr1_over_the_storage_and_lb_for_64k_and_1m_pages() {
    let storage = GuestMemory::from_regions(vec![page_table_storage()]).expect("storage");
    let d = select(&select_request(), Some("4.93"), &constructed(), &storage);
    assert_eq!(
        load_of(&d),
        Some(PageTableLoad {
            sdr1: PAGE_TABLE_STORAGE_BASE,
            lb: 0b1001,
        })
    );
}

#[test]
fn a_selection_refuses_where_no_storage_holds_the_whole_page_table() {
    let short = Region::new(
        PAGE_TABLE_STORAGE_BASE,
        0x2_0000,
        "short table",
        PageSize::Page64K,
    );
    let short = GuestMemory::from_regions(vec![short]).expect("storage");
    for storage in [GuestMemory::new(0), short] {
        let d = select(&select_request(), Some("4.93"), &constructed(), &storage);
        assert!(
            matches!(
                d,
                HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
                    what: "a selection of an address space with no storage for its page table",
                    ..
                })
            ),
            "{d:?}"
        );
    }
}
