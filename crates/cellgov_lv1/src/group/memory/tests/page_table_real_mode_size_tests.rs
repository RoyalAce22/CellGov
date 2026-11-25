use super::*;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::address_space::game_os_boot_vas_id;
use cellgov_ps3_abi::lv1::page_table::PAGE_TABLE_HANDLERS;
use cellgov_time::GuestTicks;

/// The bytes of real mode memory the test storage holds.
const REAL_MODE_HELD: u64 = 0x20_0000;

/// A valid doubleword 0 with software bit 0x40, as the 4.93 kernel's
/// insert routine makes it.
const D0: u64 = 0x0000_0012_3456_7841;

/// The doubleword 1 of a 4 KB page at 0x123000, in the real mode region.
const D1: u64 = 0x0000_0000_0012_3000;

/// The vas id that the console capture `tests/micro/lv2_vas_boot_record`
/// reads from the 4.93 kernel's record.
fn vas_id() -> u64 {
    game_os_boot_vas_id("4.93").expect("the 4.93 boot vas id")
}

/// A write of the valid entry [`D0`] and [`D1`] at index 9 of the
/// selected address space.
fn request() -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number: WRITE_HTAB_ENTRY,
        inputs: [0, 9, D0, D1, 0, 0, 0, 0],
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

/// The 4.93 answer to [`request`], with `real_mode_size` as the source of
/// the real mode size.
fn write_from(real_mode_size: fn(&str) -> Option<u64>) -> HypercallDisposition {
    write_entry_with(
        &request(),
        Some("4.93"),
        Regions {
            spaces: &spaces(),
            spes: &LogicalSpeState::default(),
            allocated: &MemoryState::default(),
            device_bus: &DeviceBusState::default(),
            real_mode_size,
        },
        &storage(),
    )
}

/// While this holds, the page table handlers do not reach their refusal
/// of a firmware whose real mode size no source gives.
#[test]
fn every_firmware_with_a_page_table_handler_row_has_a_real_mode_size() {
    let firmwares: Vec<&str> = PAGE_TABLE_HANDLERS.iter().map(|row| row.firmware).collect();
    assert!(!firmwares.is_empty(), "no page table handler row");
    let unsourced: Vec<&str> = firmwares
        .into_iter()
        .filter(|fw| memory::real_mode_size(fw).is_none())
        .collect();
    assert_eq!(unsourced, Vec::<&str>::new());
}

#[test]
fn a_valid_real_mode_entry_gets_r3_0_with_the_repository_real_mode_size() {
    let disposition = write_from(memory::real_mode_size);
    assert!(
        matches!(&disposition, HypercallDisposition::Answered(answer) if answer.r3 == 0),
        "{disposition:?}"
    );
    // The shipped entry point passes the repository source.
    assert_eq!(
        write_entry(
            &request(),
            Some("4.93"),
            &spaces(),
            &LogicalSpeState::default(),
            &MemoryState::default(),
            &DeviceBusState::default(),
            &storage(),
        ),
        disposition
    );
}

#[test]
fn a_valid_real_mode_entry_is_refused_when_no_source_gives_the_real_mode_size() {
    let r = request();
    assert_eq!(
        write_from(|_: &str| None),
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
            number: WRITE_HTAB_ENTRY,
            unit: r.unit,
            pc: r.pc,
            what: "a valid entry at a firmware whose real mode size no source gives",
        })
    );
}
