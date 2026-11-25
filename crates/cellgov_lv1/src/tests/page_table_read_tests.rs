use super::*;
use crate::address_space::AddressSpaceChange;
use crate::state::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{
    CONSTRUCT_VIRTUAL_ADDRESS_SPACE, READ_HTAB_ENTRIES, SELECT_VIRTUAL_ADDRESS_SPACE,
};
use cellgov_time::GuestTicks;

/// The vas id that the console capture `tests/micro/lv2_vas_boot_record`
/// reads from the 4.93 kernel's record.
const VAS_ID: u64 = 2;

/// The last index whose four entries a 256 KB page table holds.
const LAST_INDEX: u64 = 16380;

fn call(number: u64, r3: u64, r4: u64, r5: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs: [r3, r4, r5, 0, 0, 0, 0, 0],
        pc: 0x8000_0000_0005_d2a0,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(9),
    }
}

fn read_request(vas_id: u64, index: u64) -> HypercallRequest {
    call(READ_HTAB_ENTRIES, vas_id, index, 0)
}

/// The 4.93 kernel's address space: a 256 KB page table, constructed
/// and, with `selected`, selected.
fn spaces(selected: bool) -> AddressSpaceState {
    let mut spaces = AddressSpaceState::default();
    spaces.apply(AddressSpaceChange::Constructed(AddressSpace {
        vas_id: VAS_ID,
        htab_size_exp: 18,
        page_size_count: 2,
        page_shifts: 0x1014_0000_0000_0000,
        mapping: None,
    }));
    if selected {
        spaces.apply(AddressSpaceChange::Selected(VAS_ID));
    }
    spaces
}

/// Doubleword 0 of entry `index` in [`storage`].
fn doubleword0(index: u64) -> u64 {
    0x0000_1000_0000_0001 | (index << 7)
}

/// Doubleword 1 of entry `index` in [`storage`].
fn doubleword1(index: u64) -> u64 {
    0x0000_0000_0040_0000 | (index & 0xFFF) | 0x0180
}

/// The page table storage with entries 0 to 11 and the last four set.
fn storage() -> GuestMemory {
    let mut mem = GuestMemory::from_regions(vec![page_table_storage()]).expect("storage");
    for index in (0..12).chain(LAST_INDEX..LAST_INDEX + 4) {
        let mut bytes = doubleword0(index).to_be_bytes().to_vec();
        bytes.extend_from_slice(&doubleword1(index).to_be_bytes());
        let range = ByteRange::new(GuestAddr::new(entry_address(index)), 16).expect("range");
        mem.apply_commit(range, &bytes).expect("commit");
    }
    mem
}

fn expected(first: u64) -> HypercallDisposition {
    let entries: Vec<u64> = (first..first + 4).collect();
    let mut outputs: Vec<u64> = entries.iter().map(|&i| doubleword0(i)).collect();
    let halfwords = entries
        .iter()
        .fold(0, |packed, &i| (packed << 16) | (doubleword1(i) & 0xFFFF));
    outputs.extend([halfwords, 0, 0]);
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(0, outputs))
}

fn refusal_what(d: &HypercallDisposition) -> Option<&'static str> {
    match d {
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput { what, .. }) => Some(what),
        _ => None,
    }
}

#[test]
fn a_read_of_the_selected_table_returns_four_entries_as_storage_holds_them() {
    let (spaces, storage) = (spaces(true), storage());
    for vas_id in [0, VAS_ID] {
        for first in [0, 6, LAST_INDEX] {
            let d = read_entries(
                &read_request(vas_id, first),
                Some("4.93"),
                &spaces,
                &storage,
            );
            assert_eq!(d, expected(first), "vas id {vas_id} index {first}");
        }
    }
}

#[test]
fn an_index_whose_entries_pass_the_table_end_answers_lv1_index_status() {
    let (spaces, storage) = (spaces(true), storage());
    for index in [LAST_INDEX + 1, 16384, u64::MAX - 4] {
        let d = read_entries(&read_request(0, index), Some("4.93"), &spaces, &storage);
        assert_eq!(d, lv1_answer(READ_HTAB_INDEX_R3), "index {index}");
    }
}

#[test]
fn an_index_whose_end_overflows_refuses() {
    let (spaces, storage) = (spaces(true), storage());
    for index in [u64::MAX - 3, u64::MAX] {
        let d = read_entries(&read_request(0, index), Some("4.93"), &spaces, &storage);
        assert_eq!(
            refusal_what(&d),
            Some("a page table read whose index plus 4 overflows a doubleword"),
            "index {index}"
        );
    }
}

#[test]
fn a_read_with_no_address_space_or_storage_refuses() {
    let storage = storage();
    let cases = [
        (spaces(true), 3, Some("4.93")),
        (spaces(true), 0, None),
        (spaces(true), 0, Some("4.92")),
    ];
    assert_eq!(cases.len(), 3);
    for (spaces, vas_id, firmware) in cases {
        let d = read_entries(&read_request(vas_id, 0), firmware, &spaces, &storage);
        assert!(
            refusal_what(&d).is_some(),
            "{firmware:?} vas id {vas_id}: {d:?}"
        );
    }
    let d = read_entries(
        &read_request(0, 0),
        Some("4.93"),
        &spaces(true),
        &GuestMemory::new(0),
    );
    assert_eq!(
        refusal_what(&d),
        Some("a page table read with no storage for the table")
    );
}

#[test]
fn the_hypervisor_reads_the_table_only_from_the_storage_it_is_given() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let shifts = 0x1014_0000_0000_0000;
    let storage = storage();
    hv.dispatch_with(
        &call(CONSTRUCT_VIRTUAL_ADDRESS_SPACE, 0, 2, shifts),
        &storage,
    );
    hv.dispatch_with(&call(SELECT_VIRTUAL_ADDRESS_SPACE, VAS_ID, 0, 0), &storage);
    assert_eq!(hv.address_spaces().selected(), Some(VAS_ID));
    assert_eq!(hv.dispatch_with(&read_request(0, 0), &storage), expected(0));
    let d = hv.dispatch(&read_request(0, 0));
    assert_eq!(
        refusal_what(&d),
        Some("a page table read with no storage for the table")
    );
}
