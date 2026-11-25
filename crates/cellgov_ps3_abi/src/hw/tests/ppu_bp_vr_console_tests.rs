//! BP_VR held to the console capture of the 4.93 boot repository record.

use super::CELL_BP_VR;
use crate::lv1::repository::{BOOT_RECORD_4_93, BOOT_RECORD_CAPTURE};

/// The offset in the 4.93 boot record of the word that the kernel's
/// `stw` of its `mfspr` of BP_VR writes.
const BP_VR_OFFSET: u64 = 0xb8;

/// The doubleword that the capture read at `address`.
fn captured_doubleword(address: u64) -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(BOOT_RECORD_CAPTURE)
        .join("cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let len = u32::from_be_bytes(frame[4..8].try_into().expect("4 bytes")) as usize;
    assert_eq!(frame.len(), 8 + len, "one whole frame");
    let pairs: Vec<(u64, u64)> = frame[8..]
        .chunks_exact(16)
        .map(|pair| {
            let word =
                |at: usize| u64::from_be_bytes(pair[at..at + 8].try_into().expect("8 bytes"));
            (word(0), word(8))
        })
        .collect();
    assert_eq!(pairs.len(), 27);
    let hits: Vec<u64> = pairs
        .iter()
        .filter(|(at, _)| *at == address)
        .map(|(_, value)| *value)
        .collect();
    assert_eq!(hits.len(), 1, "the capture reads {address:#018x} once");
    hits[0]
}

#[test]
fn the_bp_vr_value_is_the_word_the_4_93_kernel_stored_in_its_boot_record() {
    let doubleword = captured_doubleword(BOOT_RECORD_4_93 + BP_VR_OFFSET);
    // The kernel's stw writes the high word. The low word is zero fill.
    assert_eq!(doubleword, u64::from(CELL_BP_VR) << 32);
}
