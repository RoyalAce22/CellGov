//! Reads the committed console captures of kernel records: frames of
//! (address, doubleword) pairs that a probe read through the CFW's lv2
//! peek.

use crate::lv1::repository::BOOT_RECORD_CAPTURE;

/// The reads of the boot repository record's `cgov_frame.bin`: the
/// probe's three locator words and the record's 24 words.
const READS: usize = 27;

/// The frame header: the magic `CGOV` and the big-endian payload length.
const HEADER: usize = 8;

/// One read: the big-endian address and the big-endian doubleword.
const PAIR: usize = 16;

/// The doubleword that the boot repository record capture read at
/// `address`.
///
/// # Panics
///
/// Panics as [`captured_doubleword_in`] does.
pub(crate) fn captured_doubleword(address: u64) -> u64 {
    captured_doubleword_in(BOOT_RECORD_CAPTURE, READS, address)
}

/// The doubleword that the capture in the directory `capture`, of `reads`
/// reads, read at `address`.
///
/// # Panics
///
/// Panics, with the path of the frame, in these cases:
/// - the frame is missing or malformed;
/// - the frame does not hold `reads` reads;
/// - the capture does not read `address` exactly once.
pub(crate) fn captured_doubleword_in(capture: &str, reads: usize, address: u64) -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(capture)
        .join("cgov_frame.bin");
    let name = path.display().to_string();
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let hits: Vec<u64> = frame_pairs(&name, &frame, reads)
        .into_iter()
        .filter(|(at, _)| *at == address)
        .map(|(_, value)| value)
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "{name}: the capture reads {address:#018x} once"
    );
    hits[0]
}

/// The (address, doubleword) pairs of `frame`, the bytes of the file
/// `name`.
///
/// # Panics
///
/// Panics, with `name`, when `frame` is not the magic, a length equal to
/// the rest of the file, and `reads` whole pairs.
fn frame_pairs(name: &str, frame: &[u8], reads: usize) -> Vec<(u64, u64)> {
    let Some((header, body)) = frame.split_at_checked(HEADER) else {
        panic!(
            "{name}: {} bytes, short of the {HEADER}-byte frame header",
            frame.len()
        );
    };
    assert_eq!(&header[..4], b"CGOV", "{name}: frame magic");
    let len = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
    assert_eq!(
        u64::from(len),
        body.len() as u64,
        "{name}: the frame length is the rest of the file"
    );
    assert_eq!(
        body.len(),
        reads * PAIR,
        "{name}: {reads} whole (address, doubleword) pairs"
    );
    body.chunks_exact(PAIR)
        .map(|pair| {
            let (at, value) = pair.split_at(8);
            let word = |bytes: &[u8]| u64::from_be_bytes(bytes.try_into().expect("8 bytes"));
            (word(at), word(value))
        })
        .collect()
}

#[cfg(test)]
#[path = "boot_record_capture_tests.rs"]
mod frame_tests;
