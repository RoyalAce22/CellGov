use super::*;
use crate::{answered_numbers, Hypervisor};
use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::call::CONFIGURE_IRQ_STATE_BITMAP;
use cellgov_time::GuestTicks;

/// The logical PPE id of the 4.93 GameOS partition, `bi.pu` at PU index 0.
const PPE_ID: u64 = 1;

/// The `sc` of the kernel's mask update for a record whose state is 3
/// [OFW-4.93 lv2_kernel a:0x800000000000807c].
const MASK_UPDATE_SC: u64 = 0x8000_0000_0000_807c;

/// The bitmap of the 4.93 kernel's thread 0: +0x1C0 of its record, which
/// the capture `lv2_logical_ppe_record` places at 0x8000000000368f80.
const THREAD_0_BITMAP: u64 = 0x0036_9140;

/// The kernel's call from hardware thread `caller`, with the logical PPE
/// id and the thread `named` from a thread's record.
fn request(caller: Option<u8>, named: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..2].copy_from_slice(&[PPE_ID, named]);
    HypercallRequest {
        unit: UnitId::new(1),
        number: DID_UPDATE_INTERRUPT_MASK,
        inputs,
        pc: MASK_UPDATE_SC,
        lr: 0,
        thread: caller,
        problem_state: false,
        tick: GuestTicks::new(64_420),
    }
}

fn with(r: &HypercallRequest, reg: usize, value: u64) -> HypercallRequest {
    let mut changed = *r;
    changed.inputs[reg - 3] = value;
    changed
}

fn lv1_r3(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: DID_UPDATE_INTERRUPT_MASK,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

#[test]
fn a_call_that_names_the_calling_thread_answers_r3_0_and_clears_r4_to_r10() {
    for thread in 0..2u8 {
        let r = request(Some(thread), u64::from(thread));
        assert_eq!(update_mask(&r, Some("4.93")), lv1_r3(0), "thread {thread}");
    }
}

#[test]
fn the_answer_ignores_r5_to_r10() {
    let mut r = request(Some(0), 0);
    r.inputs[2..].copy_from_slice(&[5, 6, 7, 8, 9, 10]);
    assert_eq!(update_mask(&r, Some("4.93")), lv1_r3(0));
}

#[test]
fn an_r3_that_is_not_the_logical_ppe_id_gets_lv1s_no_entry_with_r3_marked() {
    for ppe_id in [0, 2, u64::MAX] {
        let r = with(&request(Some(0), 0), 3, ppe_id);
        assert_eq!(
            update_mask(&r, Some("4.93")),
            lv1_r3(INTERRUPT_MASK_PPE_ID_R3),
            "{ppe_id}"
        );
    }
}

#[test]
fn an_r4_not_below_two_gets_lv1s_no_entry_with_r4_marked() {
    for thread in [2, 0xFF, 0x1_0000_0000, u64::MAX] {
        let r = request(Some(0), thread);
        assert_eq!(
            update_mask(&r, Some("4.93")),
            lv1_r3(INTERRUPT_MASK_THREAD_R3),
            "{thread:#x}"
        );
    }
}

#[test]
fn the_ppe_id_is_checked_before_the_thread() {
    let r = with(&request(Some(0), 2), 3, 0);
    assert_eq!(
        update_mask(&r, Some("4.93")),
        lv1_r3(INTERRUPT_MASK_PPE_ID_R3)
    );
}

#[test]
fn a_call_that_names_the_other_thread_is_a_refusal() {
    for (caller, named) in [(0u8, 1u64), (1, 0)] {
        let r = request(Some(caller), named);
        assert_eq!(
            update_mask(&r, Some("4.93")),
            refused(
                &r,
                "an interrupt mask update for the other hardware thread, which lv1 signals with an interprocessor interrupt",
            ),
            "caller {caller}"
        );
    }
}

#[test]
fn a_call_from_a_unit_with_no_hardware_thread_is_a_refusal() {
    let r = request(None, 0);
    assert_eq!(
        update_mask(&r, Some("4.93")),
        refused(
            &r,
            "an interrupt mask update from a unit that names no hardware thread"
        )
    );
}

#[test]
fn a_firmware_whose_handler_is_not_read_is_a_refusal() {
    let r = request(Some(0), 0);
    for firmware in [None, Some("4.92")] {
        assert_eq!(
            update_mask(&r, firmware),
            refused(
                &r,
                "an interrupt mask update at a firmware whose lv1 handler is not read"
            ),
            "{firmware:?}"
        );
    }
}

#[test]
fn the_hypervisor_answers_31_and_keeps_its_interrupt_state() {
    assert!(answered_numbers().contains(&DID_UPDATE_INTERRUPT_MASK));
    let storage = GuestMemory::new(0x0037_0000);
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let mut bitmap = request(Some(0), 0);
    bitmap.number = CONFIGURE_IRQ_STATE_BITMAP;
    bitmap.inputs[2] = THREAD_0_BITMAP;
    assert!(matches!(
        hv.dispatch_with(&bitmap, &storage),
        HypercallDisposition::Answered(_)
    ));
    let before = hv.clone();
    let disposition = hv.dispatch_with(&request(Some(0), 0), &storage);
    assert_eq!(disposition, lv1_r3(0));
    assert_eq!(hv, before);
}
