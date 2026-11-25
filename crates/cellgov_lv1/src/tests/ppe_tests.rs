use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::GET_LOGICAL_PPE_ID;
use cellgov_time::GuestTicks;

/// The 4.93 kernel's call: `sc 1` [OFW-4.93 lv2_kernel a:0x8000000000007d2c].
fn request(thread: Option<u8>) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number: GET_LOGICAL_PPE_ID,
        inputs: [0; 8],
        pc: 0x8000_0000_0000_7d2c,
        lr: 0,
        thread,
        problem_state: false,
        tick: GuestTicks::new(52_088),
    }
}

/// lv1 returns r3 = 0 [OFW-4.93 lv1 a:0x2E9024] and the id in r4
/// [OFW-4.93 lv1 a:0x2E8FC4], and writes no other slot. The 4.93 boot
/// record capture gives the id, 1.
fn answered_id_1() -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(0, vec![1, 0, 0, 0, 0, 0, 0]))
}

#[test]
fn the_4_93_call_gets_r3_0_and_logical_ppe_id_1_with_r5_to_r10_cleared() {
    assert_eq!(
        logical_ppe_id(&request(Some(0)), Some("4.93")),
        answered_id_1()
    );
}

#[test]
fn each_hardware_thread_gets_the_one_logical_ppe_id() {
    for thread in [Some(0), Some(1), None] {
        assert_eq!(
            logical_ppe_id(&request(thread), Some("4.93")),
            answered_id_1(),
            "{thread:?}"
        );
    }
}

#[test]
fn values_in_r3_to_r10_do_not_change_the_answer() {
    let mut r = request(Some(0));
    r.inputs = [u64::MAX; 8];
    assert_eq!(logical_ppe_id(&r, Some("4.93")), answered_id_1());
}

#[test]
fn a_call_at_a_firmware_whose_lv1_handler_is_not_read_is_refused() {
    let r = request(Some(0));
    for firmware in [Some("3.70"), Some("4.92"), None] {
        assert_eq!(
            logical_ppe_id(&r, firmware),
            HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
                number: GET_LOGICAL_PPE_ID,
                unit: r.unit,
                pc: r.pc,
                what: "a logical PPE id call at a firmware whose lv1 handler is not read",
            }),
            "{firmware:?}"
        );
    }
}

#[test]
fn the_hypervisor_answers_69_and_keeps_no_witness_or_state_for_it() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let before = hv.clone();
    assert_eq!(hv.dispatch(&request(Some(0))), answered_id_1());
    assert!(hv.witnesses().is_empty());
    assert_eq!(hv.refusal(), None);
    assert_eq!(hv.sync_term(), before.sync_term());
    assert_eq!(hv, before);
}
