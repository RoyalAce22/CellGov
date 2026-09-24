//! Wire shape of the host-write record.

use super::codec::*;
use super::error::*;
use super::reasons::*;
use super::trace_record::*;
use crate::level::TraceLevel;

fn roundtrip(record: TraceRecord) {
    let mut buf = Vec::new();
    record.encode(&mut buf);
    let (decoded, used) = TraceRecord::decode(&buf).expect("full record decodes");
    assert_eq!(decoded, record);
    assert_eq!(used, buf.len());
}

#[test]
fn host_write_roundtrip_each_writer() {
    use strum::VariantArray;
    for (i, w) in HostWriter::VARIANTS.iter().enumerate() {
        roundtrip(TraceRecord::HostWrite {
            writer: *w,
            space: i as u32,
            addr: 0x8000_0000_0001_0000,
            len: 8,
            reservations_cleared: 2,
        });
    }
}

#[test]
fn host_write_tag_is_0x0e() {
    let r = TraceRecord::HostWrite {
        writer: HostWriter::Lv2Effect,
        space: 0,
        addr: 0,
        len: 0,
        reservations_cleared: 0,
    };
    let mut buf = Vec::new();
    r.encode(&mut buf);
    assert_eq!(buf[0], 0x0e);
    assert_eq!(
        buf.len(),
        22,
        "documented wire size: 1 tag + 1 writer + 4 space + 8 addr + 4 len + 4 cleared"
    );
    assert_eq!(r.level(), TraceLevel::Commits);
}

#[test]
fn host_write_boundary_values_roundtrip() {
    roundtrip(TraceRecord::HostWrite {
        writer: HostWriter::Lv2Effect,
        space: 0,
        addr: 0,
        len: 0,
        reservations_cleared: 0,
    });
    roundtrip(TraceRecord::HostWrite {
        writer: HostWriter::SharedViewSeed,
        space: u32::MAX,
        addr: u64::MAX,
        len: u32::MAX,
        reservations_cleared: u32::MAX,
    });
}

#[test]
fn host_writer_discriminants_locked() {
    assert_eq!(HostWriter::Lv2Effect as u8, 0);
    assert_eq!(HostWriter::WakeContinuation as u8, 1);
    assert_eq!(HostWriter::DmaCompletion as u8, 2);
    assert_eq!(HostWriter::RsxMirror as u8, 3);
    assert_eq!(HostWriter::SharedViewFanout as u8, 4);
    assert_eq!(HostWriter::SharedViewSeed as u8, 5);
    assert_eq!(HostWriter::SyscallOutParam as u8, 6);
    assert_eq!(HostWriter::Placement as u8, 7);
}

#[test]
fn host_writer_values_are_dense_and_the_next_value_is_free() {
    use strum::VariantArray;
    for (i, w) in HostWriter::VARIANTS.iter().enumerate() {
        assert_eq!(usize::from(u8::from(*w)), i, "{w:?} is out of order");
    }
    assert_eq!(
        HostWriter::VARIANTS.len(),
        8,
        "a writer was added or removed: pin its value in host_writer_discriminants_locked"
    );
    let past_end = HostWriter::VARIANTS.len() as u8;
    assert_eq!(
        HostWriter::try_from(past_end),
        Err(DecodeError::UnknownHostWriter(past_end))
    );
}

#[test]
fn unknown_host_writer_returns_error() {
    let mut buf = vec![TAG_HOST_WRITE, 99];
    buf.extend(std::iter::repeat_n(0u8, 20));
    assert_eq!(
        TraceRecord::decode(&buf),
        Err(DecodeError::UnknownHostWriter(99))
    );
}

#[test]
fn the_writer_byte_past_the_last_variant_is_rejected_by_decode() {
    // An off-by-one in the range check shows up at the adjacent byte;
    // a far-away value like 99 leaves it hidden.
    use strum::VariantArray;
    let past_end = HostWriter::VARIANTS.len() as u8;
    let mut buf = vec![TAG_HOST_WRITE, past_end];
    buf.extend(std::iter::repeat_n(0u8, 20));
    assert_eq!(
        TraceRecord::decode(&buf),
        Err(DecodeError::UnknownHostWriter(past_end))
    );
}

#[test]
fn a_truncated_host_write_is_truncated_not_an_unknown_writer() {
    // The length gate runs before decode reads the writer byte, so a
    // stream that is both short and malformed names the truncation.
    let buf = vec![TAG_HOST_WRITE, 99];
    assert_eq!(TraceRecord::decode(&buf), Err(DecodeError::Truncated));
}
