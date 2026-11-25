use super::*;
use crate::{answered_numbers, Hypervisor};
use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_time::GuestTicks;

/// The `sc` of the kernel's port setup for an interrupt object of type 1
/// [OFW-4.93 lv2_kernel a:0x8000000000096438].
const PORT_SETUP_SC: u64 = 0x8000_0000_0009_6438;

const NO_ID: &str = "an io irq outlet call whose outlet id no source gives";

fn request(interrupt: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[0] = interrupt;
    HypercallRequest {
        unit: UnitId::new(1),
        number: CONSTRUCT_IO_IRQ_OUTLET,
        inputs,
        pc: PORT_SETUP_SC,
        lr: 0,
        thread: Some(1),
        problem_state: false,
        tick: GuestTicks::new(50_006),
    }
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: CONSTRUCT_IO_IRQ_OUTLET,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

#[test]
fn an_interrupt_id_above_63_gets_lv1s_no_entry_with_r3_marked_and_r4_to_r10_cleared() {
    for interrupt in [64, 0xFF, 0x1_0000_0000, u64::MAX] {
        let r = request(interrupt);
        assert_eq!(
            construct_outlet(&r, Some("4.93")),
            HypercallDisposition::Answered(HypercallAnswer::lv1_status(
                IO_IRQ_OUTLET_ID_R3,
                [0; 7]
            )),
            "{interrupt:#x}"
        );
    }
}

#[test]
fn an_interrupt_id_up_to_63_is_a_refusal_that_names_the_unsourced_outlet_id() {
    for interrupt in [0, 0x0A, 0x10, 63] {
        let r = request(interrupt);
        assert_eq!(
            construct_outlet(&r, Some("4.93")),
            refused(&r, NO_ID),
            "{interrupt:#x}"
        );
    }
}

#[test]
fn a_firmware_whose_handler_is_not_read_is_a_refusal_before_the_id_check() {
    for interrupt in [0x0A, 64] {
        let r = request(interrupt);
        for firmware in [None, Some("4.92")] {
            assert_eq!(
                construct_outlet(&r, firmware),
                refused(
                    &r,
                    "an io irq outlet call at a firmware whose lv1 handler is not read"
                ),
                "{interrupt:#x} {firmware:?}"
            );
        }
    }
}

#[test]
fn the_hypervisor_dispatches_120_to_the_handler_and_keeps_no_state() {
    assert!(answered_numbers().contains(&CONSTRUCT_IO_IRQ_OUTLET));
    let storage = GuestMemory::new(0x1000);
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let before = hv.clone();
    let r = request(64);
    assert_eq!(
        hv.dispatch_with(&r, &storage),
        HypercallDisposition::Answered(HypercallAnswer::lv1_status(IO_IRQ_OUTLET_ID_R3, [0; 7]))
    );
    assert_eq!(hv, before);
    let r = request(0x0A);
    assert_eq!(hv.dispatch_with(&r, &storage), refused(&r, NO_ID));
    assert_eq!(hv.groups(), before.groups());
}
