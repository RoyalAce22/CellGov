use super::*;
use crate::AddressSpaceChange;
use cellgov_event::UnitId;
use cellgov_ps3_abi::hw::ppu_mmu::{PPE_PTE_LP, PTE_C, PTE_G, PTE_H, PTE_I, PTE_L, PTE_PP, PTE_R};
use cellgov_ps3_abi::lv1::address_space::game_os_boot_vas_id;
use cellgov_ps3_abi::lv1::call::WRITE_HTAB_ENTRY;
use cellgov_ps3_abi::lv1::page_table::PTE_M;
use cellgov_time::GuestTicks;

/// The vas id that the console capture `tests/micro/lv2_vas_boot_record`
/// reads from the 4.93 kernel's record.
fn vas_id() -> u64 {
    game_os_boot_vas_id("4.93").expect("the 4.93 boot vas id")
}

/// The bytes of real mode memory the test storage holds.
const REAL_MODE_HELD: u64 = 0x20_0000;

/// A valid doubleword 0 with software bit 0x40, as the 4.93 kernel's
/// insert routine makes it.
const D0: u64 = 0x0000_0012_3456_7841;

fn write_request(vas_id: u64, index: u64, d0: u64, d1: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number: WRITE_HTAB_ENTRY,
        inputs: [vas_id, index, d0, d1, 0, 0, 0, 0],
        pc: 0x8000_0000_0005_d5f0,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(9),
    }
}

/// The 4.93 kernel's address space, selected: a 256 KB page table with
/// page shifts 16 and 20.
fn spaces() -> AddressSpaceState {
    let mut spaces = AddressSpaceState::default();
    spaces.apply(AddressSpaceChange::Constructed(AddressSpace {
        vas_id: vas_id(),
        htab_size_exp: 18,
        page_size_count: 2,
        page_shifts: 0x1014_0000_0000_0000,
        mapping: None,
    }));
    spaces.apply(AddressSpaceChange::Selected(vas_id()));
    spaces
}

/// The page table storage and the first 2 MB of real mode memory.
fn storage() -> GuestMemory {
    GuestMemory::from_regions(vec![
        Region::new(0, REAL_MODE_HELD as usize, "real mode", PageSize::Page64K),
        page_table_storage(),
    ])
    .expect("storage")
}

/// `storage` with doubleword 0 `d0` at entry `index`.
fn with_entry(mut storage: GuestMemory, index: u64, d0: u64) -> GuestMemory {
    let range = ByteRange::new(
        GuestAddr::new(entry_address(PAGE_TABLE_STORAGE_BASE, index)),
        8,
    )
    .expect("range");
    storage
        .apply_commit(range, &d0.to_be_bytes())
        .expect("commit");
    storage
}

fn write(d0: u64, d1: u64, index: u64, storage: &GuestMemory) -> HypercallDisposition {
    write_entry(
        &write_request(0, index, d0, d1),
        Some("4.93"),
        &spaces(),
        &LogicalSpeState::default(),
        &MemoryState::default(),
        &DeviceBusState::default(),
        storage,
    )
}

/// The answer that writes doubleword 0 `d0` and doubleword 1 `stored`
/// at entry `index`.
fn written(index: u64, d0: u64, stored: u64) -> HypercallDisposition {
    let mut bytes = d0.to_be_bytes().to_vec();
    bytes.extend_from_slice(&stored.to_be_bytes());
    let mut answer = HypercallAnswer::lv1_status(0, [0; 7]);
    answer.effects.push(Effect::SharedWriteIntent {
        range: ByteRange::new(
            GuestAddr::new(entry_address(PAGE_TABLE_STORAGE_BASE, index)),
            16,
        )
        .expect("range"),
        bytes: WritePayload::new(bytes),
        source: UnitId::new(1),
        source_time: GuestTicks::new(9),
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
fn a_small_page_entry_stores_the_real_page_with_wimg_forced_to_m() {
    let d1 = 0x0000_0000_0012_3000 | PTE_R | PTE_C | PTE_I | PTE_G | PTE_PP;
    let stored = 0x0012_3000 | PTE_R | PTE_C | PTE_M | PTE_PP;
    assert_eq!(write(D0, d1, 9, &storage()), written(9, D0, stored));
}

#[test]
fn large_page_entries_store_the_large_page_bit_of_their_size_index() {
    let d0 = D0 | PTE_L;
    let one_mb = 0x0000_0000_0010_0000 | PPE_PTE_LP | 0x2;
    assert_eq!(
        write(d0, one_mb, 16, &storage()),
        written(16, d0, one_mb | PTE_M)
    );
    let sixty_four_kb = 0x0000_0000_0003_0000 | 0x2;
    assert_eq!(
        write(d0, sixty_four_kb, 17, &storage()),
        written(17, d0, sixty_four_kb | PTE_M)
    );
}

#[test]
fn each_failed_check_answers_its_marked_r3_in_lv1_order() {
    let bad_d0 = D0 | (1 << 63);
    let bad_d1 = 0x4000_0000_0001_0000;
    let full_field = 0x0000_0000_0017_F000;
    let cases = [
        (16384, bad_d0, bad_d1, WRITE_HTAB_INDEX_R3),
        (0, bad_d0, bad_d1, WRITE_HTAB_DOUBLEWORD1_R3),
        (0, D0 | PTE_L, full_field, WRITE_HTAB_DOUBLEWORD1_R3),
        (0, bad_d0, 0x1000 | 0x800, WRITE_HTAB_DOUBLEWORD1_R3),
        (0, D0, 0x1000 | 0x400, WRITE_HTAB_DOUBLEWORD1_R3),
        (0, bad_d0, 0x1_0000, WRITE_HTAB_DOUBLEWORD0_R3),
        (0, D0 | PTE_L, 0x0010_3000, WRITE_HTAB_PAGE_SIZE_R3),
        (0, D0 | PTE_L, 0x0001_8000, WRITE_HTAB_ALIGNMENT_R3),
        (0, D0 | PTE_L, 0x0018_1000, WRITE_HTAB_ALIGNMENT_R3),
    ];
    assert_eq!(cases.len(), 9);
    for (index, d0, d1, r3) in cases {
        assert_eq!(
            write(d0, d1, index, &storage()),
            lv1_answer(r3),
            "index {index} d0 {d0:#x} d1 {d1:#x}"
        );
    }
}

#[test]
fn a_small_page_with_the_large_page_field_full_passes_the_doubleword_1_check() {
    let d1 = 0x0000_0000_0007_F000;
    assert_eq!(write(D0, d1, 0, &storage()), written(0, D0, d1 | PTE_M));
}

#[test]
fn a_valid_entry_that_duplicates_another_answers_duplicate_entry() {
    let index = 0x10 + 3;
    let own = with_entry(storage(), 0x10 + 6, D0 ^ 0x38);
    assert_eq!(
        write(D0, 0x1000, index, &own),
        lv1_answer(WRITE_HTAB_DUPLICATE_R3)
    );
    let other = with_entry(storage(), other_group(index, 18) + 1, D0 | PTE_H);
    assert_eq!(
        write(D0, 0x1000, index, &other),
        lv1_answer(WRITE_HTAB_DUPLICATE_R3)
    );
}

#[test]
fn an_entry_that_differs_or_sits_in_the_written_slot_is_no_duplicate() {
    let index = 0x10 + 3;
    let cases = [
        with_entry(storage(), index, D0),
        with_entry(storage(), 0x10 + 6, D0 | PTE_H),
        with_entry(storage(), 0x10 + 6, D0 & !1),
        with_entry(storage(), other_group(index, 18), D0),
    ];
    assert_eq!(cases.len(), 4);
    for storage in &cases {
        assert_eq!(
            write(D0, 0x1000, index, storage),
            written(index, D0, 0x1000 | PTE_M)
        );
    }
}

#[test]
fn an_invalid_entry_maps_no_page_and_stores_real_page_0() {
    let d0 = D0 & !1;
    let d1 = 0x0000_0000_7FF0_0000 | PTE_R | 0x78 | 0x3;
    assert_eq!(
        write(d0, d1, 4, &storage()),
        written(4, d0, PTE_R | PTE_M | 0x3)
    );
    let d1 = 0x0000_0000_7FF0_0000 | PPE_PTE_LP;
    assert_eq!(
        write(d0 | PTE_L, d1, 4, &storage()),
        written(4, d0 | PTE_L, PPE_PTE_LP | PTE_M)
    );
}

#[test]
fn a_valid_entry_whose_page_cellgov_does_not_hold_refuses() {
    let rm_size = memory::real_mode_size("4.93").expect("4.93 real mode size");
    assert!(rm_size > REAL_MODE_HELD);
    let cases = [
        (
            rm_size,
            "a valid entry whose page lies outside the partition's real mode region",
        ),
        (
            REAL_MODE_HELD,
            "a valid entry whose page CellGov holds no storage for",
        ),
    ];
    for (address, what) in cases {
        let d = write(D0, address, 0, &storage());
        assert_eq!(refusal_what(&d), Some(what), "address {address:#x}");
    }
}

#[test]
fn a_write_with_no_address_space_or_storage_refuses() {
    let unread = "a page table call at a firmware whose lv1 handler is not read";
    let cases = [
        (
            spaces(),
            3,
            Some("4.93"),
            "a page table call on an address space that this hypervisor did not construct",
        ),
        (spaces(), 0, None, unread),
        (spaces(), 0, Some("4.92"), unread),
    ];
    assert_eq!(cases.len(), 3);
    for (spaces, vas_id, firmware, what) in cases {
        let r = write_request(vas_id, 0, D0, 0x1000);
        let d = write_entry(
            &r,
            firmware,
            &spaces,
            &LogicalSpeState::default(),
            &MemoryState::default(),
            &DeviceBusState::default(),
            &storage(),
        );
        assert_eq!(
            refusal_what(&d),
            Some(what),
            "{firmware:?} vas id {vas_id}: {d:?}"
        );
    }
    let real_mode_only = GuestMemory::from_regions(vec![Region::new(
        0,
        REAL_MODE_HELD as usize,
        "real mode",
        PageSize::Page64K,
    )])
    .expect("storage");
    assert_eq!(
        refusal_what(&write(D0, 0x1000, 0, &real_mode_only)),
        Some("a page table write with no storage for the table")
    );
    assert_eq!(
        refusal_what(&write(D0 & !1, 0x1000, 0, &real_mode_only)),
        Some("a page table write with no storage for the table"),
        "an invalid entry skips the duplicate search"
    );
}
