//! The hypervisor answers the 4.93 kernel's device reads on bus 1 with the
//! nodes that lv1 holds on the reference console, read from the committed
//! capture `tests/micro/lv1_repository_bus`.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::repository::{field, first_field};
use cellgov_time::GuestTicks;

use crate::{HypercallDisposition, HypercallRequest, Hypervisor};

/// The `sc` of the 4.93 kernel's repository read wrapper
/// [OFW-4.93 lv2_kernel a:0x80000000000922fc].
const WRAPPER_SC: u64 = 0x8000_0000_0009_22fc;

/// The key words of device `device` of bus 1, with the last two words `n`.
pub(super) fn device(device: u64, n: [u64; 2]) -> [u64; 4] {
    [first_field(b"bus", 1), field(b"dev", device), n[0], n[1]]
}

/// A read of lpar 1's key `n` through the kernel's read wrapper, which
/// sets r3 = 1 [OFW-4.93 lv2_kernel a:0x80000000000922d4].
pub(super) fn read(n: [u64; 4]) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(0),
        number: 91,
        inputs: [1, n[0], n[1], n[2], n[3], 0, 0, 0],
        pc: WRAPPER_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

pub(super) fn dispatch_4_93(request: &HypercallRequest) -> HypercallDisposition {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    hv.dispatch(request)
}

/// The r3, r4 and r5 of the 4.93 answer to a read of key `n`.
pub(super) fn answered(n: [u64; 4]) -> (u64, u64, u64) {
    let answer = match dispatch_4_93(&read(n)) {
        HypercallDisposition::Answered(answer) => answer,
        HypercallDisposition::Refused(refusal) => panic!("{refusal}"),
    };
    let outputs = answer.outputs().expect("r4 to r10");
    (answer.r3, outputs[0], outputs[1])
}

/// The capture's `node.tsv` holds v1 = 2 and v2 = 0 for
/// `bus.1.dev.0.intr.0`. lv1 returns status 0 with v1 and v2 when it finds
/// the node [OFW-4.93 lv1 a:0x2E53AC-0x2E53B8].
#[test]
fn the_interrupt_0_read_of_device_0_answers_status_0_with_2_and_0() {
    assert_eq!(answered(device(0, [field(b"intr", 0), 0])), (0, 2, 0));
}

/// The capture's `node.tsv` holds type 4 and id 1 for device 1 of bus 1.
#[test]
fn the_type_and_id_reads_of_device_1_answer_status_0_with_4_and_1() {
    assert_eq!(answered(device(1, [field(b"type", 0), 0])), (0, 4, 0));
    assert_eq!(answered(device(1, [field(b"id", 0), 0])), (0, 1, 0));
}
