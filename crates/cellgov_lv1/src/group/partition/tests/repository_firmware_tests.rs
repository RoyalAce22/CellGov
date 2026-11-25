use super::*;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::repository::{field, first_field, BOOT_RECORD_4_93, BOOT_RECORD_CAPTURE};
use cellgov_time::GuestTicks;

/// The reads of the capture's frame: three locator words and the 24
/// words of the boot repository record.
const BOOT_RECORD_READS: usize = 27;

/// The `bi.rgntotal` value that the capture [`BOOT_RECORD_CAPTURE`] read
/// at +0x68 of the 4.93 kernel's boot repository record. The README of
/// the capture gives the offset.
fn captured_rgntotal() -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(BOOT_RECORD_CAPTURE)
        .join("cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let len = u32::from_be_bytes(frame[4..8].try_into().expect("4 bytes")) as usize;
    assert_eq!(frame.len(), 8 + len, "one whole frame");
    assert_eq!(len, BOOT_RECORD_READS * 16, "{BOOT_RECORD_READS} reads");
    let word = |b: &[u8]| u64::from_be_bytes(b.try_into().expect("8 bytes"));
    let hits: Vec<u64> = frame[8..]
        .chunks_exact(16)
        .filter(|pair| word(&pair[..8]) == BOOT_RECORD_4_93 + 0x68)
        .map(|pair| word(&pair[8..]))
        .collect();
    assert_eq!(hits.len(), 1, "{}: one read of bi.rgntotal", path.display());
    hits[0]
}

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

#[test]
fn a_bi_rgntotal_read_answers_at_4_93_and_is_a_refusal_naming_the_key_at_every_floor() {
    let r = rgntotal_read();
    assert_eq!(
        read_node(&r, Some("4.93")),
        HypercallDisposition::Answered(HypercallAnswer::success([captured_rgntotal(), 0]))
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
        HypercallDisposition::Answered(HypercallAnswer::success([captured_rgntotal(), 0]))
    );
    let mut at_1_50 = crate::Hypervisor::new();
    at_1_50.set_firmware("1.50");
    assert!(matches!(
        at_1_50.dispatch(&r),
        HypercallDisposition::Refused(HypercallRefusal::RepositoryNode { .. })
    ));
}
