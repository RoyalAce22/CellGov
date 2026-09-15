//! The store watch's record layout, spec parsing and window filter.

use super::{pack_record, StoreWatch, StoreWatchSpec, RECORD_LEN};
use crate::game::taps::error::TapError;
use crate::game::taps::record_file::RecordFile;

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
fn the_window_parses_and_its_length_is_bounded() {
    let spec = StoreWatchSpec::parse(Some("0x91fe90:0x20"), Some("w.bin"))
        .unwrap()
        .unwrap();
    assert_eq!((spec.addr, spec.len), (0x91_FE90, 0x20));
    assert_eq!(StoreWatchSpec::parse(None, Some("")).unwrap(), None);
    for bad in ["10:0", "10:10001"] {
        assert!(
            matches!(
                StoreWatchSpec::parse(Some(bad), Some("w")),
                Err(TapError::OutOfRange { .. })
            ),
            "{bad}"
        );
    }
    assert!(matches!(
        StoreWatchSpec::parse(Some("10"), Some("w")),
        Err(TapError::BadShape { .. })
    ));
    assert!(matches!(
        StoreWatchSpec::parse(Some("10:4"), None),
        Err(TapError::Unpaired { .. })
    ));
}

#[test]
fn a_window_past_4_gib_is_refused_not_truncated() {
    let last = StoreWatchSpec::parse(Some("ffff0000:10000"), Some("w"))
        .unwrap()
        .unwrap();
    assert_eq!((last.addr, last.len), (0xFFFF_0000, 0x1_0000));
    for bad in ["ffff0001:10000", "100000000:4", "ffffffffffffffff:1"] {
        assert!(
            matches!(
                StoreWatchSpec::parse(Some(bad), Some("w")),
                Err(TapError::OutOfRange { .. })
            ),
            "{bad}"
        );
    }
    assert!(matches!(
        StoreWatchSpec::parse(Some("100000000:4"), Some("w")),
        Err(TapError::OutOfRange {
            value: 0x1_0000_0000,
            ..
        })
    ));
}

#[test]
fn the_header_names_the_window() {
    let spec = StoreWatchSpec::parse(Some("1000:20"), Some("w"))
        .unwrap()
        .unwrap();
    let h = spec.header();
    assert_eq!(&h[0..4], b"CGSW");
    assert_eq!(&h[4..8], &1u32.to_le_bytes());
    assert_eq!(&h[8..12], &0x1000u32.to_le_bytes());
    assert_eq!(&h[12..16], &0x20u32.to_le_bytes());
}

fn watch() -> StoreWatch<Vec<u8>> {
    let spec = StoreWatchSpec::parse(Some("1000:10"), Some("w"))
        .unwrap()
        .unwrap();
    StoreWatch::new(&spec, RecordFile::over("test", Vec::new(), &[]).unwrap())
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
