//! The value sample's record layout, header and stride.

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};

use super::{pack_record, ValueSample, ValueSampleSpec};
use crate::taps::RecordFile;

fn spec(addr: u64, width: u32, stride: u64) -> ValueSampleSpec {
    ValueSampleSpec {
        addr,
        width,
        stride,
        path: "s".into(),
    }
}

#[test]
fn a_record_packs_the_four_fields_at_their_documented_offsets() {
    let r = pack_record(0x0102_0304_0506_0708, 2, 3, &[0xAA, 0xBB, 0xCC, 0x00]);
    assert_eq!(r.len(), 13 + 4, "13-byte prefix then width value bytes");
    assert_eq!(&r[0..8], &0x0102_0304_0506_0708u64.to_le_bytes());
    assert_eq!(r[8], 2);
    assert_eq!(&r[9..13], &3u32.to_le_bytes());
    assert_eq!(&r[13..17], &[0xAA, 0xBB, 0xCC, 0x00]);
}

fn sample(addr: u64, width: u32, stride: u64) -> ValueSample<Vec<u8>> {
    ValueSample::new(
        &spec(addr, width, stride),
        RecordFile::over(Vec::new(), &[]).unwrap(),
    )
}

#[test]
fn a_sample_lands_only_on_a_stride_boundary() {
    let mut mem = GuestMemory::new(0x1000);
    let range = ByteRange::new(GuestAddr::new(0x100), 4).unwrap();
    mem.apply_commit(range, &[1, 2, 3, 4]).unwrap();
    let mut s = sample(0x100, 4, 3);
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
    let mut s = sample(0x200, 2, 1);
    s.step(1, &mem);
    assert_eq!(s.into_inner(), pack_record(1, 0, 0, &[0, 0]));
}

#[test]
fn the_header_and_a_record_are_their_golden_bytes() {
    assert_eq!(
        spec(0x0091_FE9C, 4, 1).header(),
        [b'C', b'G', b'V', b'S', 2, 0, 0, 0, 0x9C, 0xFE, 0x91, 0, 4, 0, 0, 0]
    );
    assert_eq!(
        pack_record(7, 2, 1, &[0xAA, 0, 0, 0]),
        [7, 0, 0, 0, 0, 0, 0, 0, 2, 1, 0, 0, 0, 0xAA, 0, 0, 0]
    );
}

#[test]
fn a_short_read_keeps_its_length_and_zero_fills_the_rest() {
    let mut s = sample(0x100, 4, 1);
    s.record(9, Some(&[0x11, 0x22]));
    assert_eq!(s.into_inner(), pack_record(9, 2, 2, &[0x11, 0x22, 0, 0]));
}
