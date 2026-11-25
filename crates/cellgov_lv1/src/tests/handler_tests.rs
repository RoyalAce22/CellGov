use super::*;
use crate::{AddressSpaceState, EventReceivePortState, HypercallRefusal, IrqStateBitmapState};
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
        firmware,
        &MemoryState::default(),
        &AddressSpaceState::default(),
        &IrqStateBitmapState::default(),
        &EventReceivePortState::default(),
        &GuestMemory::new(0),
    )
}

#[test]
fn the_answered_numbers_ascend() {
    assert_eq!(ANSWERED.len(), 16);
    for pair in ANSWERED.windows(2) {
        assert!(pair[0] < pair[1], "{pair:?}");
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
    assert_eq!(checked, 1026 - ANSWERED.len());
}

#[test]
fn no_answered_number_takes_the_null_backend() {
    assert_eq!(ANSWERED.len(), 16);
    for &number in ANSWERED {
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
        HypercallDisposition::Answered(HypercallAnswer::success(vec![2]))
    );
}
