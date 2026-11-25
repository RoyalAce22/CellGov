use super::*;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::repository::{field, first_field, READ_WRAPPER_SC_4_93};
use cellgov_time::GuestTicks;

/// The return address of the 4.93 kernel's `bi.rgntotal` accessor.
const RGNTOTAL_READER: u64 = 0x8000_0000_0000_c94c;

/// The return address of the 4.93 kernel's `be.0.clock` accessor.
const BE_CLOCK_READER: u64 = 0x8000_0000_0000_c85c;

fn rgntotal_read_from(pc: u64, lr: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(0),
        number: 91,
        inputs: [
            2,
            first_field(b"bi", 0),
            field(b"rgntotal", 0),
            0,
            0,
            0,
            0,
            0,
        ],
        pc,
        lr,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

fn unobserved_refusal(r: &HypercallRequest) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::RepositorySecondValueUnobserved {
        key: key_of(r),
        unit: r.unit,
        pc: r.pc,
        lr: r.lr,
    })
}

/// The 4.93 value is the console capture
/// `tests/micro/lv2_repository_boot_nodes/ps3/cech20-cex-493`.
#[test]
fn a_read_from_the_site_that_discards_the_unobserved_second_value_answers_the_first() {
    let r = rgntotal_read_from(READ_WRAPPER_SC_4_93, RGNTOTAL_READER);
    assert_eq!(
        read_node(&r, Some("4.93")),
        HypercallDisposition::Answered(HypercallAnswer::success(vec![0x0f9a_0000, 0]))
    );
}

#[test]
fn a_read_of_an_unobserved_second_value_from_another_caller_is_a_refusal_naming_the_key() {
    let r = rgntotal_read_from(READ_WRAPPER_SC_4_93, BE_CLOCK_READER);
    assert_eq!(read_node(&r, Some("4.93")), unobserved_refusal(&r));
}

#[test]
fn a_read_of_an_unobserved_second_value_from_another_sc_is_a_refusal_naming_the_key() {
    let r = rgntotal_read_from(0x8000_0000_0000_d000, RGNTOTAL_READER);
    assert_eq!(read_node(&r, Some("4.93")), unobserved_refusal(&r));
}
