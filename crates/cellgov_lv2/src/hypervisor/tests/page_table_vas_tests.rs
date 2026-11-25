use super::*;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{MAP_HTAB, READ_HTAB_ENTRIES, WRITE_HTAB_ENTRY};
use cellgov_time::GuestTicks;

/// A call with vas id 0 in r3, index 0 in r4 and a valid small page
/// entry in r5 and r6.
fn request(number: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs: [0, 0, 0x0000_0012_3456_7841, 0x1000, 0, 0, 0, 0],
        pc: 0x8000_0000_0005_d5f0,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(9),
    }
}

/// The 4.93 kernel's address space, constructed and not selected.
fn unselected() -> AddressSpaceState {
    let mut spaces = AddressSpaceState::default();
    spaces.apply(AddressSpaceChange::Constructed(AddressSpace {
        vas_id: 2,
        htab_size_exp: 18,
        page_size_count: 2,
        page_shifts: 0x1014_0000_0000_0000,
        mapping: None,
    }));
    spaces
}

#[test]
fn vas_id_0_before_a_selection_answers_lv1_no_entry() {
    let storage = GuestMemory::from_regions(vec![
        Region::new(0, 0x20_0000, "real mode", PageSize::Page64K),
        page_table_storage(),
    ])
    .expect("storage");
    let spaces = unselected();
    let firmware = Some("4.93");
    let answers = [
        write_entry(&request(WRITE_HTAB_ENTRY), firmware, &spaces, &storage),
        read_entries(&request(READ_HTAB_ENTRIES), firmware, &spaces, &storage),
        map_table(
            &request(MAP_HTAB),
            firmware,
            &spaces,
            &MemoryState::default(),
        ),
    ];
    assert_eq!(answers.len(), 3);
    for d in answers {
        assert_eq!(d, lv1_answer(HTAB_NO_ENTRY_R3));
    }
}
