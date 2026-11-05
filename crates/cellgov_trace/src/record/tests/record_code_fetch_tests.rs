//! The wire form of an unsynchronized code fetch record.

use super::codec::*;
use super::error::*;
use super::trace_record::*;
use cellgov_event::UnitId;

fn record() -> TraceRecord {
    TraceRecord::UnsyncedCodeFetch {
        unit: UnitId::new(1),
        pc: 0x0001_0144,
        space: 2,
        line: 0x0001_0100,
        writer: UnitId::new(3),
        store_step: 4,
        sequence: SEQUENCE_DCBST | SEQUENCE_SYNC,
        patch_class: true,
    }
}

#[test]
fn the_record_encodes_its_fields_little_endian_behind_its_tag() {
    let mut buf = Vec::new();
    record().encode(&mut buf);

    let mut want = vec![TAG_UNSYNCED_CODE_FETCH];
    want.extend(1u64.to_le_bytes());
    want.extend(0x0001_0144u64.to_le_bytes());
    want.extend(2u32.to_le_bytes());
    want.extend(0x0001_0100u64.to_le_bytes());
    want.extend(3u64.to_le_bytes());
    want.extend(4u64.to_le_bytes());
    want.extend([0b011, 1]);
    assert_eq!(buf, want);
    assert_eq!(TraceRecord::decode(&buf), Ok((record(), buf.len())));
}

#[test]
fn a_patch_class_byte_other_than_zero_or_one_is_refused() {
    let mut buf = Vec::new();
    record().encode(&mut buf);
    let last = buf.len() - 1;
    buf[last] = 2;
    assert_eq!(TraceRecord::decode(&buf), Err(DecodeError::InvalidBool(2)));
}
