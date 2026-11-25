use super::*;
use crate::group::memory::ADDRESS_SPACE_SOURCE;
use crate::state::Hypervisor;
use crate::MemoryRegion;
use cellgov_event::UnitId;
use cellgov_mem::lanes::{contribution, LaneIndex};
use cellgov_ps3_abi::hw::ppu_mmu::PTE_L;
use cellgov_ps3_abi::lv1::address_space::game_os_boot_vas_id;
use cellgov_ps3_abi::lv1::call::{
    ALLOCATE_MEMORY, CONSTRUCT_VIRTUAL_ADDRESS_SPACE, MAP_HTAB, SELECT_VIRTUAL_ADDRESS_SPACE,
    WRITE_HTAB_ENTRY,
};
use cellgov_ps3_abi::lv1::memory::region_address;
use cellgov_ps3_abi::lv1::page_table::PTE_M;
use cellgov_time::GuestTicks;

/// The vas id that the console capture `tests/micro/lv2_vas_boot_record`
/// reads from the 4.93 kernel's record.
fn vas_id() -> u64 {
    game_os_boot_vas_id("4.93").expect("the 4.93 boot vas id")
}

/// The size of the 4.93 kernel's page table: 256 KB.
const TABLE_BYTES: u64 = 0x4_0000;

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

fn spaces(selected: bool) -> AddressSpaceState {
    let mut spaces = AddressSpaceState::default();
    spaces.apply(AddressSpaceChange::Constructed(AddressSpace {
        vas_id: vas_id(),
        htab_size_exp: 18,
        page_size_count: 2,
        page_shifts: 0x1014_0000_0000_0000,
        mapping: None,
    }));
    if selected {
        spaces.apply(AddressSpaceChange::Selected(vas_id()));
    }
    spaces
}

fn boot_counter() -> u64 {
    game_os_region_counter_at_boot("4.93").expect("4.93 boot counter")
}

fn mapped(counter: u64) -> HypercallDisposition {
    let lpar_address = region_address(TABLE_BYTES, counter).expect("address");
    let mut answer = HypercallAnswer::lv1_status(0, [lpar_address, 0, 0, 0, 0, 0, 0]);
    answer.push_change(AddressSpaceChange::Mapped {
        vas_id: vas_id(),
        lpar_address,
        counter,
    });
    HypercallDisposition::Answered(answer)
}

fn refusal_what(d: &HypercallDisposition) -> Option<&'static str> {
    match d {
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput { what, .. }) => Some(what),
        _ => None,
    }
}

#[test]
fn a_mapping_takes_the_next_region_counter_value() {
    let fresh = MemoryState::default();
    for vas_id in [0, vas_id()] {
        let d = map_table(
            &call(MAP_HTAB, [vas_id, 0, 0, 0]),
            Some("4.93"),
            &spaces(true),
            &fresh,
            &HeldRegions::default(),
        );
        assert_eq!(d, mapped(boot_counter()), "vas id {vas_id}");
    }
    let mut allocated = MemoryState::default();
    allocated.create(MemoryRegion {
        lpar_address: region_address(0x10_0000, boot_counter()).expect("address"),
        size: 0x10_0000,
        page_size_exp: 20,
        unit_id: 0,
        counter: boot_counter(),
    });
    let d = map_table(
        &call(MAP_HTAB, [0, 0, 0, 0]),
        Some("4.93"),
        &spaces(true),
        &allocated,
        &HeldRegions::default(),
    );
    assert_eq!(d, mapped(boot_counter() + 1));
}

#[test]
fn a_mapping_refuses_twice_and_where_no_address_space_is_named() {
    let mut twice = spaces(true);
    twice.apply(AddressSpaceChange::Mapped {
        vas_id: vas_id(),
        lpar_address: 0x1234_0000,
        counter: 0,
    });
    let unread = "a page table call at a firmware whose lv1 handler is not read";
    let cases = [
        (twice, 0, Some("4.93"), "a second mapping of a page table"),
        (
            spaces(true),
            3,
            Some("4.93"),
            "a page table call on an address space that this hypervisor did not construct",
        ),
        (spaces(true), 0, None, unread),
        (spaces(true), 0, Some("4.92"), unread),
    ];
    assert_eq!(cases.len(), 4);
    for (spaces, vas_id, firmware, what) in cases {
        let d = map_table(
            &call(MAP_HTAB, [vas_id, 0, 0, 0]),
            firmware,
            &spaces,
            &MemoryState::default(),
            &HeldRegions::default(),
        );
        assert_eq!(
            refusal_what(&d),
            Some(what),
            "{firmware:?} vas id {vas_id}: {d:?}"
        );
    }
}

#[test]
fn the_page_table_storage_lies_above_the_4_93_real_mode_region() {
    let rm_size = memory::real_mode_size("4.93").expect("4.93 real mode size");
    assert!(rm_size <= PAGE_TABLE_STORAGE_BASE, "{rm_size:#x}");
}

#[test]
fn the_hypervisor_keeps_the_mapping_and_maps_its_pages_to_the_table_storage() {
    let storage = GuestMemory::from_regions(vec![page_table_storage()]).expect("storage");
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let shifts = 0x1014_0000_0000_0000;
    hv.dispatch_with(
        &call(CONSTRUCT_VIRTUAL_ADDRESS_SPACE, [0, 2, shifts, 0]),
        &storage,
    );
    hv.dispatch_with(
        &call(SELECT_VIRTUAL_ADDRESS_SPACE, [vas_id(), 0, 0, 0]),
        &storage,
    );
    let d = hv.dispatch_with(&call(MAP_HTAB, [0, 0, 0, 0]), &storage);
    assert_eq!(d, mapped(boot_counter()));
    let lpar = region_address(TABLE_BYTES, boot_counter()).expect("address");
    assert_eq!(hv.address_spaces().spaces()[0].mapping, Some(lpar));
    assert_eq!(hv.memory().next_counter(), Some(boot_counter() + 1));

    let d0 = 0x0000_0012_3456_7801 | PTE_L;
    let page = lpar + 0x1_0000;
    let d = hv.dispatch_with(&call(WRITE_HTAB_ENTRY, [0, 8, d0, page | 0x2]), &storage);
    let HypercallDisposition::Answered(answer) = d else {
        panic!("{d:?}");
    };
    let mut bytes = d0.to_be_bytes().to_vec();
    bytes.extend_from_slice(&((PAGE_TABLE_STORAGE_BASE + 0x1_0000) | PTE_M | 0x2).to_be_bytes());
    assert_eq!(
        answer.effects,
        vec![Effect::SharedWriteIntent {
            range: ByteRange::new(
                GuestAddr::new(entry_address(PAGE_TABLE_STORAGE_BASE, 8)),
                16
            )
            .expect("range"),
            bytes: WritePayload::new(bytes),
            source: UnitId::new(1),
            source_time: GuestTicks::new(9),
        }]
    );

    let past = hv.dispatch_with(
        &call(
            WRITE_HTAB_ENTRY,
            [0, 9, d0 ^ 0x80, (lpar + TABLE_BYTES) | 0x2],
        ),
        &storage,
    );
    assert_eq!(
        refusal_what(&past),
        Some("a valid entry whose page lies outside the partition's real mode region")
    );

    let allocate = call(ALLOCATE_MEMORY, [0x10_0000, 20, 0, 0]);
    let HypercallDisposition::Answered(answer) = hv.dispatch_with(&allocate, &storage) else {
        panic!("the allocation refused");
    };
    assert_eq!(
        answer.outputs().expect("r4 to r10")[0],
        region_address(0x10_0000, boot_counter() + 1).expect("address")
    );
}

#[test]
fn a_mapping_adds_its_address_as_lane_5_of_the_address_space_term() {
    let unmapped = spaces(true);
    assert_eq!(unmapped.spaces().len(), 1);
    let lpar_address = 0x0004_8000_0000_0000;
    let mut mapped = spaces(true);
    mapped.apply(AddressSpaceChange::Mapped {
        vas_id: vas_id(),
        lpar_address,
        counter: 0x20,
    });
    let lane_5 = contribution(LaneIndex::new(ADDRESS_SPACE_SOURCE, 0, 5, 0), lpar_address);
    assert_ne!(lane_5, 0);
    assert_eq!(
        mapped.sync_term(),
        unmapped.sync_term().wrapping_add(lane_5)
    );
}
