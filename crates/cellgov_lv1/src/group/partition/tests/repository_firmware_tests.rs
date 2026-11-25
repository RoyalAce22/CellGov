use super::*;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::repository::{field, first_field};
use cellgov_time::GuestTicks;

/// The 4.93 kernel's `bi.rgntotal` read: the wrapper's `sc`
/// [OFW-4.93 lv2_kernel a:0x800000000000ce3c], called from the accessor
/// that discards the second value [OFW-4.93 lv2_kernel a:0x800000000000c94c].
fn rgntotal_read() -> HypercallRequest {
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
        pc: 0x8000_0000_0000_ce3c,
        lr: 0x8000_0000_0000_c94c,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

/// The 4.93 value is the console capture
/// `tests/micro/lv2_repository_boot_nodes/ps3/cech20-cex-493`.
#[test]
fn a_bi_rgntotal_read_answers_at_4_93_and_is_a_refusal_naming_the_key_at_every_floor() {
    let r = rgntotal_read();
    assert_eq!(
        read_node(&r, Some("4.93")),
        HypercallDisposition::Answered(HypercallAnswer::success([0x0f9a_0000, 0]))
    );
    for fw in ["1.50", "1.94", "3.70"] {
        assert_eq!(
            read_node(&r, Some(fw)),
            HypercallDisposition::Refused(HypercallRefusal::RepositoryNode {
                key: key_of(&r),
                unit: r.unit,
                pc: r.pc,
            }),
            "{fw}"
        );
    }
}

#[test]
fn the_hypervisor_answers_a_read_at_the_firmware_it_was_set_to() {
    let r = rgntotal_read();
    let mut at_4_93 = crate::Hypervisor::new();
    at_4_93.set_firmware("4.93");
    assert_eq!(
        at_4_93.dispatch(&r),
        HypercallDisposition::Answered(HypercallAnswer::success([0x0f9a_0000, 0]))
    );
    let mut at_1_50 = crate::Hypervisor::new();
    at_1_50.set_firmware("1.50");
    assert!(matches!(
        at_1_50.dispatch(&r),
        HypercallDisposition::Refused(HypercallRefusal::RepositoryNode { .. })
    ));
}
