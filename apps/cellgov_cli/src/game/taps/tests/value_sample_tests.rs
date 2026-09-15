//! The value sample's record layout, spec parsing and stride.

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};

use super::{pack_record, ValueSample, ValueSampleSpec};
use crate::game::taps::error::TapError;
use crate::game::taps::record_file::RecordFile;

#[test]
fn a_record_packs_the_four_fields_at_their_documented_offsets() {
    let r = pack_record(0x0102_0304_0506_0708, 2, 3, &[0xAA, 0xBB, 0xCC, 0x00]);
    assert_eq!(r.len(), 13 + 4, "13-byte prefix then width value bytes");
    assert_eq!(&r[0..8], &0x0102_0304_0506_0708u64.to_le_bytes());
    assert_eq!(r[8], 2);
    assert_eq!(&r[9..13], &3u32.to_le_bytes());
    assert_eq!(&r[13..17], &[0xAA, 0xBB, 0xCC, 0x00]);
}

#[test]
fn the_range_and_stride_parse_and_are_bounded() {
    let spec = ValueSampleSpec::parse(Some("0x91FE9C:4"), Some("s.bin"), Some("16"))
        .unwrap()
        .unwrap();
    assert_eq!((spec.addr, spec.width, spec.stride), (0x91_FE9C, 4, 16));
    let default_stride = ValueSampleSpec::parse(Some("10:100"), Some("s"), None)
        .unwrap()
        .unwrap();
    assert_eq!((default_stride.width, default_stride.stride), (256, 1));
    for (range, stride) in [("10:0", None), ("10:101", None), ("10:4", Some("0"))] {
        assert!(
            matches!(
                ValueSampleSpec::parse(Some(range), Some("s"), stride),
                Err(TapError::OutOfRange { .. })
            ),
            "{range} {stride:?}"
        );
    }
    assert!(matches!(
        ValueSampleSpec::parse(Some("10:4"), Some("s"), Some("x")),
        Err(TapError::BadNumber { .. })
    ));
    assert!(matches!(
        ValueSampleSpec::parse(None, Some("s"), None),
        Err(TapError::Unpaired { .. })
    ));
}

#[test]
fn a_stride_with_no_range_or_path_is_refused() {
    assert!(matches!(
        ValueSampleSpec::parse(None, None, Some("16")),
        Err(TapError::Unpaired { .. })
    ));
    assert_eq!(ValueSampleSpec::parse(None, None, Some(" ")).unwrap(), None);
}

#[test]
fn an_address_past_32_bits_is_refused_not_truncated() {
    let last = ValueSampleSpec::parse(Some("ffffffff:1"), Some("s"), None)
        .unwrap()
        .unwrap();
    assert_eq!(last.addr, 0xFFFF_FFFF);
    assert!(matches!(
        ValueSampleSpec::parse(Some("100000000:4"), Some("s"), None),
        Err(TapError::OutOfRange {
            value: 0x1_0000_0000,
            ..
        })
    ));
}

fn sample(spec: &str, stride: &str) -> ValueSample<Vec<u8>> {
    let spec = ValueSampleSpec::parse(Some(spec), Some("s"), Some(stride))
        .unwrap()
        .unwrap();
    ValueSample::new(&spec, RecordFile::over("test", Vec::new(), &[]).unwrap())
}

#[test]
fn a_sample_lands_only_on_a_stride_boundary() {
    let mut mem = GuestMemory::new(0x1000);
    let range = ByteRange::new(GuestAddr::new(0x100), 4).unwrap();
    mem.apply_commit(range, &[1, 2, 3, 4]).unwrap();
    let mut s = sample("100:4", "3");
    for step in 1..=6 {
        s.step(step, &mem);
    }
    let out = s.into_inner();
    assert_eq!(
        out,
        [
            pack_record(3, 1, 4, &[1, 2, 3, 4]),
            pack_record(6, 1, 4, &[1, 2, 3, 4])
        ]
        .concat()
    );
}

#[test]
fn an_unmapped_range_records_status_zero_and_a_zeroed_value() {
    let mem = GuestMemory::new(0x100);
    let mut s = sample("200:2", "1");
    s.step(1, &mem);
    assert_eq!(s.into_inner(), pack_record(1, 0, 0, &[0, 0]));
}
