use super::*;
use cellgov_event::UnitId;
use cellgov_time::GuestTicks;

fn request(number: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(3),
        number,
        inputs: [0; 8],
        pc: 0x8000_0000_0000_fc54,
        thread: Some(1),
        tick: GuestTicks::new(10),
    }
}

#[test]
fn the_answered_numbers_ascend() {
    assert_eq!(ANSWERED.len(), 5);
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
            answer(&r),
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
    assert_eq!(ANSWERED.len(), 5);
    for &number in ANSWERED {
        let d = answer(&request(number));
        assert!(
            !matches!(
                d,
                HypercallDisposition::Refused(HypercallRefusal::Unmodelled { .. })
            ),
            "{number}: {d:?}"
        );
    }
}

/// psdevwiki's "Hypervisor Reverse Engineering" page puts GameOS in
/// LPAR 2.
#[test]
fn get_logical_partition_id_answers_lpar_2_in_r4() {
    let d = answer(&request(74));
    assert_eq!(
        d,
        HypercallDisposition::Answered(HypercallAnswer::success(vec![2]))
    );
}
