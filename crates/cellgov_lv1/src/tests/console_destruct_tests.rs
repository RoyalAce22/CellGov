use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{LOGICAL_CONSOLE_CONSTRUCT, LOGICAL_CONSOLE_DESTRUCT};
use cellgov_time::GuestTicks;

/// The address of the 4.93 kernel's `sc` for hypercall 106
/// [OFW-4.93 lv2_kernel a:0x80000000000104e8].
const KERNEL_DESTRUCT_PC: u64 = 0x8000_0000_0001_04e8;

fn request(number: u64, console: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs: [console, 7, 7, 7, 7, 7, 7, 7],
        pc: KERNEL_DESTRUCT_PC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(5),
    }
}

fn gate_off_answer() -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(0, vec![0; 7]))
}

#[test]
fn a_4_93_destruct_answers_0_in_r3_and_clears_r4_to_r10_for_any_console_id() {
    for console in [0, 1, 0xFFF, 0x1000, u64::MAX] {
        let r = request(LOGICAL_CONSOLE_DESTRUCT, console);
        assert_eq!(destruct(&r, Some("4.93")), gate_off_answer(), "{console}");
    }
}

#[test]
fn a_destruct_at_a_firmware_with_no_gate_capture_is_a_refusal() {
    let r = request(LOGICAL_CONSOLE_DESTRUCT, 1);
    for firmware in [None, Some("4.92"), Some("3.55")] {
        let d = destruct(&r, firmware);
        assert!(
            matches!(
                d,
                HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
                    number: LOGICAL_CONSOLE_DESTRUCT,
                    unit,
                    pc,
                    ..
                }) if unit == r.unit && pc == r.pc
            ),
            "{firmware:?}: {d:?}"
        );
    }
}

#[test]
fn a_construct_after_a_destruct_answers_as_the_first_and_no_state_changes() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let before = hv.sync_term();
    let first = hv.dispatch(&request(LOGICAL_CONSOLE_CONSTRUCT, 1));
    let destruct = hv.dispatch(&request(LOGICAL_CONSOLE_DESTRUCT, 1));
    let second = hv.dispatch(&request(LOGICAL_CONSOLE_CONSTRUCT, 1));
    assert!(
        matches!(&first, HypercallDisposition::Answered(a) if a.r3 == 0),
        "{first:?}"
    );
    assert_eq!(destruct, gate_off_answer());
    assert_eq!(second, first);
    assert_eq!(hv.refusal(), None);
    assert_eq!(hv.sync_term(), before);
    assert!(hv.console_text().is_empty());
    assert!(hv.witnesses().is_empty());
}
