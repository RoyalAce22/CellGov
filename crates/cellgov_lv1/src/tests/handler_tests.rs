use super::*;
use crate::{HypercallAnswer, HypercallRefusal};
use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_time::GuestTicks;

fn request(number: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(3),
        number,
        inputs: [0; 8],
        pc: 0x8000_0000_0000_fc54,
        lr: 0,
        thread: Some(1),
        problem_state: false,
        tick: GuestTicks::new(10),
    }
}

fn answer(r: &HypercallRequest, firmware: Option<&str>) -> HypercallDisposition {
    super::answer(
        r,
        CallerTranslation::Loads,
        firmware,
        &GroupStates::default(),
        &GuestMemory::new(0),
    )
}

#[test]
fn the_answered_numbers_are_the_registrations_ascending() {
    let answered = answered_numbers();
    assert_eq!(answered.len(), table::registrations().count());
    assert!(!answered.is_empty(), "no group registers a hypercall");
    for pair in answered.windows(2) {
        assert!(pair[0] < pair[1], "{pair:?}");
    }
    for number in &answered {
        assert!(answers(*number), "{number}");
    }
}

#[test]
fn every_number_with_no_handler_takes_the_null_backend_by_number() {
    let mut checked = 0;
    for number in (0..=1024).chain([u64::MAX]) {
        if answers(number) {
            continue;
        }
        let r = request(number);
        assert_eq!(
            answer(&r, None),
            HypercallDisposition::Refused(HypercallRefusal::Unmodelled {
                number,
                unit: r.unit,
                pc: r.pc,
            })
        );
        checked += 1;
    }
    assert_eq!(checked, 1026 - answered_numbers().len());
}

#[test]
fn no_answered_number_takes_the_null_backend() {
    let answered = answered_numbers();
    assert_eq!(answered.len(), table::registrations().count());
    for number in answered {
        let d = answer(&request(number), None);
        assert!(
            !matches!(
                d,
                HypercallDisposition::Refused(HypercallRefusal::Unmodelled { .. })
            ),
            "{number}: {d:?}"
        );
    }
}

/// The system manager enables a device interface for GameOS through the
/// bit of LPAR 2 [PS3DEVWIKI-HV-RE s:Enabling_WLAN_Gelic_On_FAT].
#[test]
fn get_logical_partition_id_answers_lpar_2_in_r4() {
    let d = answer(&request(74), None);
    assert_eq!(
        d,
        HypercallDisposition::Answered(HypercallAnswer::success([2]))
    );
}
