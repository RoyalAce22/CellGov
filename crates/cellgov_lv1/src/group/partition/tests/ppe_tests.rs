use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::GET_LOGICAL_PPE_ID;
use cellgov_ps3_abi::lv1::ppe::LOGICAL_PPE_RECORD_CAPTURE;
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

/// The reads of the capture's frame: three locator words, the record
/// count and 13 words of each of the two per-thread records.
const RECORD_READS: usize = 30;

/// The address of +0x08 of the 640-byte record of hardware thread
/// `thread`, where the kernel stores the r4 of its boot call. The README
/// of [`LOGICAL_PPE_RECORD_CAPTURE`] gives the layout.
fn record_id_address(thread: u64) -> u64 {
    0x8000_0000_0036_8f80 + 640 * thread + 0x08
}

/// The logical PPE id that the capture [`LOGICAL_PPE_RECORD_CAPTURE`]
/// reads in the record of each hardware thread. Both reads must agree.
fn captured_id() -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(LOGICAL_PPE_RECORD_CAPTURE)
        .join("cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let len = u32::from_be_bytes(frame[4..8].try_into().expect("4 bytes")) as usize;
    assert_eq!(frame.len(), 8 + len, "one whole frame");
    assert_eq!(len, RECORD_READS * 16, "{RECORD_READS} reads");
    let word = |b: &[u8]| u64::from_be_bytes(b.try_into().expect("8 bytes"));
    let read_at = |address: u64| {
        let hits: Vec<u64> = frame[8..]
            .chunks_exact(16)
            .filter(|pair| word(&pair[..8]) == address)
            .map(|pair| word(&pair[8..]))
            .collect();
        assert_eq!(
            hits.len(),
            1,
            "{}: one read of {address:#x}",
            path.display()
        );
        hits[0]
    };
    let id = read_at(record_id_address(0));
    assert_eq!(read_at(record_id_address(1)), id, "the record of thread 1");
    id
}

/// lv1 returns r3 = 0 [OFW-4.93 lv1 a:0x2E9024] and the id in r4
/// [OFW-4.93 lv1 a:0x2E8FC4], and writes no other slot.
fn answered_captured_id() -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(
        0,
        [captured_id(), 0, 0, 0, 0, 0, 0],
    ))
}

#[test]
fn the_4_93_call_gets_r3_0_and_the_captured_logical_ppe_id_with_r5_to_r10_cleared() {
    assert_eq!(
        logical_ppe_id(&request(Some(0)), Some("4.93")),
        answered_captured_id()
    );
}

#[test]
fn each_hardware_thread_gets_the_one_logical_ppe_id() {
    for thread in [Some(0), Some(1), None] {
        assert_eq!(
            logical_ppe_id(&request(thread), Some("4.93")),
            answered_captured_id(),
            "{thread:?}"
        );
    }
}

#[test]
fn values_in_r3_to_r10_do_not_change_the_answer() {
    let mut r = request(Some(0));
    r.inputs = [u64::MAX; 8];
    assert_eq!(logical_ppe_id(&r, Some("4.93")), answered_captured_id());
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
    assert_eq!(hv.dispatch(&request(Some(0))), answered_captured_id());
    assert!(hv.witnesses().is_empty());
    assert_eq!(hv.refusal(), None);
    assert_eq!(hv.sync_term(), before.sync_term());
    assert_eq!(hv, before);
}
