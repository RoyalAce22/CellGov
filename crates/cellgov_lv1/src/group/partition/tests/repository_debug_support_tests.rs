use super::*;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::repository::{field, first_field};
use cellgov_time::GuestTicks;

/// The 4.93 kernel's `ss.param.debug.support` read: the `sc`
/// [OFW-4.93 lv2_kernel a:0x8000000000050a50], called from
/// [OFW-4.93 lv2_kernel a:0x8000000000050a9c].
fn debug_support_read() -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(0),
        number: 91,
        inputs: [
            1,
            first_field(b"ss", 0),
            field(b"param", 0),
            field(b"debug", 0),
            field(b"support", 0),
            0,
            0,
            0,
        ],
        pc: 0x8000_0000_0005_0a50,
        lr: 0x8000_0000_0005_0aa0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

#[test]
fn a_debug_support_read_is_a_refusal_naming_the_key_at_every_floor() {
    let r = debug_support_read();
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
