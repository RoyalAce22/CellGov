//! The registered `allocate_memory` and `map_htab` handlers skip the
//! wrapped counter address that a region of the partition holds.

use crate::group::state::GroupStates;
use crate::{
    AddressSpace, AddressSpaceChange, HypercallAnswer, HypercallDisposition, HypercallRequest,
    MemoryChange, MemoryRegion,
};
use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::address_space::game_os_boot_vas_id;
use cellgov_ps3_abi::lv1::call::{ALLOCATE_MEMORY, MAP_HTAB};
use cellgov_ps3_abi::lv1::memory::{
    game_os_region_counter_at_boot, region_address, ALLOCATE_MEMORY_UNIT_ID,
};
use cellgov_time::GuestTicks;

/// 1 MB: e = 20, so the address holds 2^22 counter values.
const MB: u64 = 0x10_0000;

/// The size of the 4.93 kernel's page table, 256 KB: e = 18, so the
/// address holds 2^24 counter values.
const TABLE_BYTES: u64 = 0x4_0000;

fn boot() -> u64 {
    game_os_region_counter_at_boot("4.93").expect("4.93 boot counter")
}

fn address(size: u64, counter: u64) -> u64 {
    region_address(size, counter).expect("an address")
}

fn call(number: u64, inputs: [u64; 4]) -> HypercallRequest {
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

/// The disposition from the handler that the table registers for the
/// request's number.
fn dispatch(groups: &GroupStates, request: &HypercallRequest) -> HypercallDisposition {
    crate::handlers::answer(
        request,
        crate::CallerTranslation::Loads,
        Some("4.93"),
        groups,
        &GuestMemory::new(0),
    )
}

/// An allocated region of `size` bytes at the boot value's address, and
/// a counter whose next value is the boot value plus the count of values
/// that the address holds, `values`.
fn held_boot_region(size: u64, page_size_exp: u8, values: u64) -> GroupStates {
    let mut groups = GroupStates::default();
    groups.apply(
        MemoryRegion {
            lpar_address: address(size, boot()),
            size,
            page_size_exp,
            unit_id: ALLOCATE_MEMORY_UNIT_ID,
            counter: boot(),
        }
        .into(),
    );
    groups.apply(MemoryChange::CounterTaken(values + boot() - 1).into());
    groups
}

#[test]
fn an_allocation_whose_wrapped_address_a_region_holds_takes_the_next_counter_value() {
    let values = 1 << 22;
    let groups = held_boot_region(MB, 20, values);
    assert_eq!(
        groups.memory().regions().next_counter(),
        Some(values + boot())
    );
    assert_eq!(address(MB, values + boot()), address(MB, boot()));
    let lpar_address = address(MB, boot() + 1);
    let mut answer = HypercallAnswer::success([lpar_address, ALLOCATE_MEMORY_UNIT_ID]);
    answer.push_change(MemoryRegion {
        lpar_address,
        size: MB,
        page_size_exp: 20,
        unit_id: ALLOCATE_MEMORY_UNIT_ID,
        counter: values + boot() + 1,
    });
    assert_eq!(
        dispatch(&groups, &call(ALLOCATE_MEMORY, [MB, 20, 0, 0])),
        HypercallDisposition::Answered(answer)
    );
}

#[test]
fn a_page_table_mapping_whose_wrapped_address_a_region_holds_takes_the_next_counter_value() {
    let values = 1 << 24;
    let vas_id = game_os_boot_vas_id("4.93").expect("the 4.93 boot vas id");
    let mut groups = held_boot_region(TABLE_BYTES, 16, values);
    groups.apply(
        AddressSpaceChange::Constructed(AddressSpace {
            vas_id,
            htab_size_exp: 18,
            page_size_count: 2,
            page_shifts: 0x1014_0000_0000_0000,
            mapping: None,
        })
        .into(),
    );
    assert_eq!(
        groups.memory().regions().next_counter(),
        Some(values + boot())
    );
    assert_eq!(
        address(TABLE_BYTES, values + boot()),
        address(TABLE_BYTES, boot())
    );
    let lpar_address = address(TABLE_BYTES, boot() + 1);
    let mut answer = HypercallAnswer::success([lpar_address]);
    answer.push_change(AddressSpaceChange::Mapped {
        vas_id,
        lpar_address,
        counter: values + boot() + 1,
    });
    assert_eq!(
        dispatch(&groups, &call(MAP_HTAB, [vas_id, 0, 0, 0])),
        HypercallDisposition::Answered(answer)
    );
}
