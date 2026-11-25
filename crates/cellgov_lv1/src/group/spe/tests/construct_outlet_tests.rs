//! The construct of the 4.93 kernel's boot SPE gives the interrupt group
//! the SPE's three irq outlets, with the ids that the capture
//! `lv2_logical_spe_object` reads.

use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{CONSTRUCT_VIRTUAL_ADDRESS_SPACE, SET_SPE_TRANSITION_NOTIFIER};
use cellgov_time::GuestTicks;

/// A thread 0 call of hypercall `number` at the `sc` address `pc`.
fn call(number: u64, pc: u64, inputs: [u64; 8]) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs,
        pc,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

/// A 4.93 hypervisor after the kernel's address space and its boot
/// construct [OFW-4.93 lv2_kernel a:0x800000000004f188].
fn constructed() -> Hypervisor {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    for setup in [
        call(
            CONSTRUCT_VIRTUAL_ADDRESS_SPACE,
            0,
            [0, 2, 0x1014_0000_0000_0000, 0, 0, 0, 0, 0],
        ),
        call(
            CONSTRUCT_LOGICAL_SPE,
            0x8000_0000_0004_f188,
            [12, 12, 12, 12, 12, 2, 1, 0],
        ),
    ] {
        assert!(
            matches!(hv.dispatch(&setup), HypercallDisposition::Answered(_)),
            "hypercall {}: {:?}",
            setup.number,
            hv.refusal()
        );
    }
    hv
}

fn captured_outlets() -> [u64; 3] {
    ["outlet_0", "outlet_1", "outlet_2"].map(super::capture_tests::captured)
}

#[test]
fn the_hypervisor_keeps_the_boot_spe_s_three_outlets_unconnected_and_no_port() {
    let hv = constructed();
    let held: Vec<EventReceivePort> = hv
        .event_receive_ports()
        .outlets()
        .values()
        .copied()
        .collect();
    assert_eq!(held, captured_outlets().map(EventReceivePort::new));
    assert!(hv.event_receive_ports().ports().is_empty());
}

#[test]
fn the_transition_notifier_call_refuses_an_outlet_as_its_port() {
    let mut hv = constructed();
    let spe = super::capture_tests::captured("spe_id");
    for outlet in captured_outlets() {
        let r = call(
            SET_SPE_TRANSITION_NOTIFIER,
            0,
            [spe, 1, outlet, 0, 0, 0, 0, 0],
        );
        assert_eq!(
            hv.dispatch(&r),
            HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
                number: SET_SPE_TRANSITION_NOTIFIER,
                unit: r.unit,
                pc: r.pc,
                what: "an SPE transition notifier call on a port id that names no event receive \
                       port the hypervisor constructed",
            }),
            "outlet {outlet}"
        );
    }
}
