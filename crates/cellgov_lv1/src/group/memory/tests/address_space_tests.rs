use super::*;
use crate::state::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{CONSTRUCT_VIRTUAL_ADDRESS_SPACE, SELECT_VIRTUAL_ADDRESS_SPACE};
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's construction
/// [OFW-4.93 lv2_kernel a:0x800000000005dcf4].
const PC: u64 = 0x8000_0000_0005_dcf4;

/// The vas id and the page table size that the console capture
/// `tests/micro/lv2_vas_boot_record` reads from the 4.93 kernel's
/// record.
const CAPTURED_VAS_ID: u64 = 2;
const CAPTURED_HTAB_SIZE: u64 = 0x4_0000;

fn request(number: u64, inputs: [u64; 3]) -> HypercallRequest {
    let mut all = [0; 8];
    all[..3].copy_from_slice(&inputs);
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs: all,
        pc: PC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(9),
    }
}

fn construct_request(r3: u64, r4: u64, r5: u64) -> HypercallRequest {
    request(CONSTRUCT_VIRTUAL_ADDRESS_SPACE, [r3, r4, r5])
}

/// The call the 4.93 kernel makes
/// [OFW-4.93 lv2_kernel a:0x800000000005dcc0-0x800000000005dcf4]: r3 = 0,
/// two page sizes, shifts 16 and 20.
fn kernel_call() -> HypercallRequest {
    construct_request(0, KERNEL_PAGE_SIZE_COUNT, KERNEL_PAGE_SHIFTS)
}

fn select_request(vas_id: u64) -> HypercallRequest {
    request(SELECT_VIRTUAL_ADDRESS_SPACE, [vas_id, 0, 0])
}

fn boot_space() -> AddressSpace {
    AddressSpace {
        vas_id: CAPTURED_VAS_ID,
        htab_size_exp: 18,
        page_size_count: 2,
        page_shifts: 0x1014_0000_0000_0000,
        mapping: None,
    }
}

fn constructed() -> AddressSpaceState {
    let mut spaces = AddressSpaceState::default();
    spaces.apply(AddressSpaceChange::Constructed(boot_space()));
    spaces
}

/// A real address space that holds the page table storage.
fn storage() -> GuestMemory {
    GuestMemory::from_regions(vec![page_table::page_table_storage()]).expect("storage")
}

fn lv1_answer(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

const HTAB_SIZE_REFUSAL: &str = "a page table size that no kernel call is shown to pass";
const PAGE_SIZES_REFUSAL: &str = "page sizes that no kernel call is shown to pass";
const SECOND_CONSTRUCTION_REFUSAL: &str =
    "a second address space construction, whose vas id no source gives";
const CONSTRUCT_FIRMWARE_REFUSAL: &str =
    "an address space construction at a firmware whose lv1 handler is not read";
const UNCONSTRUCTED_SELECTION_REFUSAL: &str =
    "a selection of an address space that this hypervisor did not construct";
const SELECT_FIRMWARE_REFUSAL: &str =
    "an address space selection at a firmware whose lv1 handler is not read";

/// Whether `disposition` is the unmodelled-input refusal `expected`.
fn refused_for(disposition: &HypercallDisposition, expected: &str) -> bool {
    matches!(
        disposition,
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput { what, .. })
            if *what == expected
    )
}

#[test]
fn the_kernel_call_answers_the_captured_vas_id_and_page_table_size() {
    let d = construct(&kernel_call(), Some("4.93"), &AddressSpaceState::default());
    let mut expected =
        HypercallAnswer::lv1_status(0, [CAPTURED_VAS_ID, CAPTURED_HTAB_SIZE, 0, 0, 0, 0, 0]);
    expected.push_change(AddressSpaceChange::Constructed(boot_space()));
    assert_eq!(d, HypercallDisposition::Answered(expected));
}

#[test]
fn an_exponent_of_eighteen_or_below_answers_as_the_kernel_call_does() {
    let kernel = construct(&kernel_call(), Some("4.93"), &AddressSpaceState::default());
    assert!(
        matches!(kernel, HypercallDisposition::Answered(_)),
        "{kernel:?}"
    );
    for r3 in [1, 17, 18] {
        let d = construct(
            &construct_request(r3, KERNEL_PAGE_SIZE_COUNT, KERNEL_PAGE_SHIFTS),
            Some("4.93"),
            &AddressSpaceState::default(),
        );
        assert_eq!(d, kernel, "r3 {r3}");
    }
}

#[test]
fn each_failed_lv1_check_answers_its_marked_r3_in_lv1_order() {
    let none = AddressSpaceState::default();
    let cases = [
        (21, 2, KERNEL_PAGE_SHIFTS, 0x1_FFFF_FFEF),
        (u64::MAX, 3, 0, 0x1_FFFF_FFEF),
        (0, 3, KERNEL_PAGE_SHIFTS, 0x2_FFFF_FFEF),
        (0, u64::MAX, 0, 0x2_FFFF_FFEF),
        (0, 2, 0x100C_0000_0000_0000, 0x4_FFFF_FFEF),
        (0, 1, 0x0C00_0000_0000_0000, 0x4_FFFF_FFEF),
    ];
    assert_eq!(cases.len(), 6);
    for (r3, r4, r5, status) in cases {
        let d = construct(&construct_request(r3, r4, r5), Some("4.93"), &none);
        assert_eq!(d, lv1_answer(status), "r3 {r3} r4 {r4} r5 {r5:#x}");
    }
}

#[test]
fn inputs_that_pass_lv1_checks_outside_the_kernel_call_refuse() {
    let none = AddressSpaceState::default();
    let cases = [
        (19, 2, KERNEL_PAGE_SHIFTS, HTAB_SIZE_REFUSAL),
        (20, 2, KERNEL_PAGE_SHIFTS, HTAB_SIZE_REFUSAL),
        (0, 0, 0, PAGE_SIZES_REFUSAL),
        (0, 1, 0x1000_0000_0000_0000, PAGE_SIZES_REFUSAL),
        (0, 2, 0x1018_0000_0000_0000, PAGE_SIZES_REFUSAL),
        (0, 2, 0x1014_0000_0000_0001, PAGE_SIZES_REFUSAL),
    ];
    assert_eq!(cases.len(), 6);
    for (r3, r4, r5, what) in cases {
        let d = construct(&construct_request(r3, r4, r5), Some("4.93"), &none);
        assert!(refused_for(&d, what), "r3 {r3} r4 {r4} r5 {r5:#x}: {d:?}");
    }
}

#[test]
fn a_second_construction_refuses() {
    let d = construct(&kernel_call(), Some("4.93"), &constructed());
    assert!(refused_for(&d, SECOND_CONSTRUCTION_REFUSAL), "{d:?}");
}

#[test]
fn a_construction_refuses_where_no_lv1_read_gives_the_handler() {
    let failing_check = construct_request(21, KERNEL_PAGE_SIZE_COUNT, KERNEL_PAGE_SHIFTS);
    for firmware in [None, Some("4.92"), Some("3.41")] {
        for r in [kernel_call(), failing_check] {
            let d = construct(&r, firmware, &AddressSpaceState::default());
            assert!(
                refused_for(&d, CONSTRUCT_FIRMWARE_REFUSAL),
                "{firmware:?} r3 {}: {d:?}",
                r.r(3)
            );
        }
    }
}

#[test]
fn selecting_the_constructed_address_space_answers_with_outputs_cleared() {
    let d = select(
        &select_request(CAPTURED_VAS_ID),
        CallerTranslation::Loads,
        Some("4.93"),
        &constructed(),
        &storage(),
    );
    let mut expected = HypercallAnswer::lv1_status(0, [0; 7]);
    expected.push_change(AddressSpaceChange::Selected(CAPTURED_VAS_ID));
    expected.page_table = Some(page_table::load(&boot_space()));
    assert_eq!(d, HypercallDisposition::Answered(expected));
}

#[test]
fn selecting_an_address_space_the_hypervisor_did_not_construct_refuses() {
    for vas_id in [1, 3, u64::MAX] {
        let d = select(
            &select_request(vas_id),
            CallerTranslation::Loads,
            Some("4.93"),
            &constructed(),
            &storage(),
        );
        assert!(
            refused_for(&d, UNCONSTRUCTED_SELECTION_REFUSAL),
            "vas id {vas_id}: {d:?}"
        );
    }
    let d = select(
        &select_request(CAPTURED_VAS_ID),
        CallerTranslation::Loads,
        Some("4.93"),
        &AddressSpaceState::default(),
        &storage(),
    );
    assert!(refused_for(&d, UNCONSTRUCTED_SELECTION_REFUSAL), "{d:?}");
}

#[test]
fn selecting_vas_id_zero_refuses_as_the_partition_default_address_space() {
    let d = select(
        &select_request(0),
        CallerTranslation::Loads,
        Some("4.93"),
        &constructed(),
        &storage(),
    );
    assert!(
        matches!(
            d,
            HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
                what: "a selection of the partition's default address space",
                ..
            })
        ),
        "{d:?}"
    );
}

#[test]
fn a_selection_refuses_where_no_lv1_read_gives_the_handler() {
    for firmware in [None, Some("4.92")] {
        let d = select(
            &select_request(CAPTURED_VAS_ID),
            CallerTranslation::Loads,
            firmware,
            &constructed(),
            &storage(),
        );
        assert!(
            refused_for(&d, SELECT_FIRMWARE_REFUSAL),
            "{firmware:?}: {d:?}"
        );
    }
}

#[test]
fn the_hypervisor_keeps_the_constructed_address_space_and_its_selection() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    assert!(matches!(
        hv.dispatch(&kernel_call()),
        HypercallDisposition::Answered(_)
    ));
    assert_eq!(hv.address_spaces().spaces(), &[boot_space()]);
    assert_eq!(hv.address_spaces().selected(), None);
    assert!(matches!(
        hv.dispatch_with(&select_request(CAPTURED_VAS_ID), &storage()),
        HypercallDisposition::Answered(_)
    ));
    assert_eq!(hv.address_spaces().selected(), Some(CAPTURED_VAS_ID));
    assert_eq!(hv.refusal(), None);
}
