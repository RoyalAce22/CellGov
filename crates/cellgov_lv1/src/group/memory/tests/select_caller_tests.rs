use super::*;
use crate::state::Hypervisor;
use crate::{page_table_storage, HypercallRefusal, RunningThreads, SavedMsrs};
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::SELECT_VIRTUAL_ADDRESS_SPACE;
use cellgov_time::GuestTicks;

/// The vas id that the console capture `tests/micro/lv2_vas_boot_record`
/// reads from the 4.93 kernel's record.
const CAPTURED_VAS_ID: u64 = 2;

const CALLER_REFUSAL: &str =
    "a selection from a unit that holds no SDR1 or HID6 for the page table load";

fn request(number: u64, inputs: [u64; 3]) -> HypercallRequest {
    let mut all = [0; 8];
    all[..3].copy_from_slice(&inputs);
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs: all,
        pc: 0x8000_0000_0005_dd10,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(9),
    }
}

fn storage() -> GuestMemory {
    GuestMemory::from_regions(vec![page_table_storage()]).expect("storage")
}

/// A 4.93 hypervisor that holds the address space of the 4.93 kernel's
/// construction [OFW-4.93 lv2_kernel a:0x800000000005dcf4].
fn constructed() -> Hypervisor {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let construct = request(
        CONSTRUCT_VIRTUAL_ADDRESS_SPACE,
        [0, KERNEL_PAGE_SIZE_COUNT, KERNEL_PAGE_SHIFTS],
    );
    let d = hv.dispatch_with(&construct, &storage());
    assert!(matches!(d, HypercallDisposition::Answered(_)), "{d:?}");
    hv
}

fn select_from(hv: &mut Hypervisor, caller: CallerTranslation) -> HypercallDisposition {
    hv.dispatch_signalling(
        &request(SELECT_VIRTUAL_ADDRESS_SPACE, [CAPTURED_VAS_ID, 0, 0]),
        caller,
        &storage(),
        &SavedMsrs::default(),
        &RunningThreads::default(),
    )
}

#[test]
fn a_caller_that_refuses_the_load_gets_a_refusal_and_the_selection_stays_unset() {
    let mut hv = constructed();
    let d = select_from(&mut hv, CallerTranslation::Refuses);
    let expected = HypercallRefusal::UnmodelledInput {
        number: SELECT_VIRTUAL_ADDRESS_SPACE,
        unit: UnitId::new(1),
        pc: 0x8000_0000_0005_dd10,
        what: CALLER_REFUSAL,
    };
    assert_eq!(d, HypercallDisposition::Refused(expected));
    assert_eq!(hv.refusal(), Some(expected));
    assert_eq!(hv.address_spaces().selected(), None);
}

#[test]
fn a_caller_that_takes_the_load_selects_the_address_space() {
    let mut hv = constructed();
    let d = select_from(&mut hv, CallerTranslation::Loads);
    let space = hv
        .address_spaces()
        .space(CAPTURED_VAS_ID)
        .expect("the constructed space");
    let load = page_table::load(space, page_table::PAGE_TABLE_STORAGE_BASE);
    assert!(
        matches!(&d, HypercallDisposition::Answered(a) if a.page_table.as_ref() == Some(&load)),
        "{d:?}"
    );
    assert_eq!(hv.refusal(), None);
    assert_eq!(hv.address_spaces().selected(), Some(CAPTURED_VAS_ID));
}
