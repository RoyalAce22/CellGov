//! The store watch's record layout, header and window filter.

use super::{pack_record, StoreWatch, StoreWatchSpec, RECORD_LEN};
use crate::taps::RecordFile;

fn spec(addr: u64, len: u64) -> StoreWatchSpec {
    StoreWatchSpec {
        addr,
        len,
        path: "w".into(),
    }
}

/// The patch set's hook writes this layout too. A change to the order
/// or width of a field makes one reader misread the captures of both.
#[test]
fn a_record_packs_the_five_fields_at_their_documented_offsets() {
    let r = pack_record(
        0x0102_0304_0506_0708,
        0x1122_3344,
        0x99AA_BBCC,
        0x10,
        0xDEAD_BEEF,
    );
    assert_eq!(r.len(), 28);
    assert_eq!(&r[0..8], &0x0102_0304_0506_0708u64.to_le_bytes());
    assert_eq!(&r[8..12], &0x1122_3344u32.to_le_bytes());
    assert_eq!(&r[12..16], &0x99AA_BBCCu32.to_le_bytes());
    assert_eq!(&r[16..20], &0x10u32.to_le_bytes());
    assert_eq!(&r[20..28], &0xDEAD_BEEFu64.to_le_bytes());
}
#[test]
fn a_record_keeps_the_low_32_bits_of_a_wide_effective_address() {
    let r = pack_record(0, 0, 0x0000_0001_9988_7766, 4, 0);
    assert_eq!(&r[12..16], &0x9988_7766u32.to_le_bytes());
}
#[test]
fn the_header_names_the_window() {
    let spec = spec(0x1000, 0x20);
    let h = spec.header();
    assert_eq!(&h[0..4], b"CGSW");
    assert_eq!(&h[4..8], &1u32.to_le_bytes());
    assert_eq!(&h[8..12], &0x1000u32.to_le_bytes());
    assert_eq!(&h[12..16], &0x20u32.to_le_bytes());
}

fn watch() -> StoreWatch<Vec<u8>> {
    StoreWatch::new(
        &spec(0x1000, 0x10),
        RecordFile::over(Vec::new(), &[]).unwrap(),
    )
}

#[test]
fn only_a_write_overlapping_the_window_is_recorded() {
    let mut w = watch();
    w.write(0x100, 0x0FFC, &[1, 2, 3, 4]);
    w.write(0x100, 0x1010, &[5]);
    w.write(0x100, 0x0FFE, &[6, 7, 8, 9]);
    w.write(0x104, 0x100F, &[0xA; 12]);
    let out = w.into_inner();
    assert_eq!(out.len(), 2 * RECORD_LEN);
    assert_eq!(
        &out[..RECORD_LEN],
        &pack_record(
            0,
            0x100,
            0x0FFE,
            4,
            u64::from_le_bytes([6, 7, 8, 9, 0, 0, 0, 0])
        )
    );
    assert_eq!(
        &out[RECORD_LEN..],
        &pack_record(1, 0x104, 0x100F, 12, u64::from_le_bytes([0xA; 8]))
    );
}
#[test]
fn an_empty_write_is_not_recorded() {
    let mut w = watch();
    w.write(0, 0x1000, &[]);
    assert!(w.into_inner().is_empty());
}

#[test]
fn the_header_and_a_record_are_their_golden_bytes() {
    assert_eq!(
        spec(0x0091_FE90, 0x20).header(),
        [b'C', b'G', b'S', b'W', 1, 0, 0, 0, 0x90, 0xFE, 0x91, 0, 0x20, 0, 0, 0]
    );
    assert_eq!(
        pack_record(1, 0x0001_0230, 0x0091_FE94, 4, 0x0000_0000_DDCC_BBAA),
        [
            1, 0, 0, 0, 0, 0, 0, 0, // record
            0x30, 0x02, 0x01, 0, // pc
            0x94, 0xFE, 0x91, 0, // ea
            4, 0, 0, 0, // width
            0xAA, 0xBB, 0xCC, 0xDD, 0, 0, 0, 0, // value
        ]
    );
}
