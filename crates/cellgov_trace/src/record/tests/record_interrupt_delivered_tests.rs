//! The wire form of a delivered interrupt record.

use super::codec::*;
use super::trace_record::*;
use crate::level::TraceLevel;
use cellgov_event::UnitId;
use cellgov_time::GuestTicks;

fn record() -> TraceRecord {
    TraceRecord::InterruptDelivered {
        unit: UnitId::new(1),
        vector: 0xC00,
        srr0: 0x0001_0204,
        time: GuestTicks::new(0x0506_0708),
    }
}

#[test]
fn the_record_encodes_unit_vector_srr0_and_time_little_endian_behind_its_tag() {
    let mut buf = Vec::new();
    record().encode(&mut buf);

    let mut want = vec![TAG_INTERRUPT_DELIVERED];
    want.extend(1u64.to_le_bytes());
    want.extend(0xC00u64.to_le_bytes());
    want.extend(0x0001_0204u64.to_le_bytes());
    want.extend(0x0506_0708u64.to_le_bytes());
    assert_eq!(buf, want);
    assert_eq!(TraceRecord::decode(&buf), Ok((record(), buf.len())));
}

#[test]
fn the_record_is_a_scheduling_record_with_tag_0x16() {
    assert_eq!(record().tag(), 0x16);
    assert_eq!(record().level(), TraceLevel::Scheduling);
}
