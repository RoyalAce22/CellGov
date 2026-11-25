//! The wire form of the hypercall entry, return and console records.

use super::codec::*;
use super::error::DecodeError;
use super::reasons::TracedHypercallDisposition;
use super::trace_record::*;
use crate::level::TraceLevel;
use cellgov_event::UnitId;
use cellgov_time::GuestTicks;

fn entered(thread: Option<u8>) -> TraceRecord {
    TraceRecord::HypercallEntered {
        unit: UnitId::new(2),
        number: 91,
        args: [2, 0x6269_0000, 0x7075 << 48, 0, 0, 6, 7, 8],
        pc: 0x8000_0000_0000_ce3c,
        thread,
        disposition: TracedHypercallDisposition::Refused,
        time: GuestTicks::new(0x1234),
    }
}

fn returned() -> TraceRecord {
    TraceRecord::HypercallReturned {
        unit: UnitId::new(2),
        status: (-6i64) as u64,
        outputs: [1, 2, 0, 0, 0, 0, 0],
        written: 2,
        untimed: true,
        time: GuestTicks::new(0x99),
    }
}

fn console() -> TraceRecord {
    let mut bytes = [0u8; CONSOLE_RECORD_BYTES];
    bytes[..5].copy_from_slice(b"lv2(0");
    TraceRecord::HypervisorConsole {
        unit: UnitId::new(2),
        console: 1,
        len: 5,
        bytes,
    }
}

#[test]
fn the_entry_record_encodes_its_fields_little_endian_behind_its_tag() {
    let mut buf = Vec::new();
    entered(Some(1)).encode(&mut buf);

    let mut want = vec![TAG_HYPERCALL_ENTERED];
    want.extend(2u64.to_le_bytes());
    want.extend(91u64.to_le_bytes());
    for a in [2u64, 0x6269_0000, 0x7075 << 48, 0, 0, 6, 7, 8] {
        want.extend(a.to_le_bytes());
    }
    want.extend(0x8000_0000_0000_ce3cu64.to_le_bytes());
    want.extend([1, 1, 2]);
    want.extend(0x1234u64.to_le_bytes());
    assert_eq!(buf, want);
    assert_eq!(TraceRecord::decode(&buf), Ok((entered(Some(1)), buf.len())));
}

#[test]
fn every_hypercall_record_decodes_to_itself_at_its_encoded_length() {
    let records = [entered(None), entered(Some(0)), returned(), console()];
    assert_eq!(records.len(), 4);
    for record in records {
        let mut buf = Vec::new();
        record.encode(&mut buf);
        assert_eq!(Some(buf.len()), TraceRecord::encoded_len(record.tag()));
        assert_eq!(
            TraceRecord::decode(&buf),
            Ok((record, buf.len())),
            "{record:?}"
        );
    }
}

#[test]
fn the_hypercall_disposition_values_are_locked() {
    use strum::VariantArray;
    assert_eq!(TracedHypercallDisposition::VARIANTS.len(), 3);
    assert_eq!(TracedHypercallDisposition::Answered as u8, 0);
    assert_eq!(TracedHypercallDisposition::Unmodelled as u8, 1);
    assert_eq!(TracedHypercallDisposition::Refused as u8, 2);
}

#[test]
fn an_entry_record_with_an_unknown_disposition_does_not_decode() {
    let mut buf = Vec::new();
    entered(None).encode(&mut buf);
    // The disposition byte follows the tag, unit, number, eight args, pc
    // and the two thread bytes.
    let at = 1 + 8 + 8 + 64 + 8 + 2;
    buf[at] = 3;
    assert_eq!(
        TraceRecord::decode(&buf),
        Err(DecodeError::UnknownHypercallDisposition(3))
    );
}

#[test]
fn the_hypercall_records_are_scheduling_records_with_tags_0x17_to_0x19() {
    assert_eq!(
        [entered(None).tag(), returned().tag(), console().tag()],
        [0x17, 0x18, 0x19]
    );
    for record in [entered(None), returned(), console()] {
        assert_eq!(record.level(), TraceLevel::Scheduling);
    }
}

#[test]
fn the_return_record_encodes_its_fields_little_endian_behind_its_tag() {
    let mut buf = Vec::new();
    returned().encode(&mut buf);

    let mut want = vec![TAG_HYPERCALL_RETURNED];
    want.extend(2u64.to_le_bytes());
    want.extend(((-6i64) as u64).to_le_bytes());
    for o in [1u64, 2, 0, 0, 0, 0, 0] {
        want.extend(o.to_le_bytes());
    }
    want.extend([2, 1]);
    want.extend(0x99u64.to_le_bytes());
    assert_eq!(buf, want);
}

#[test]
fn the_console_record_encodes_its_count_then_every_byte_slot() {
    let mut buf = Vec::new();
    console().encode(&mut buf);

    let mut want = vec![TAG_HYPERVISOR_CONSOLE];
    want.extend(2u64.to_le_bytes());
    want.extend(1u64.to_le_bytes());
    want.push(5);
    want.extend(b"lv2(0");
    want.extend([0u8; CONSOLE_RECORD_BYTES - 5]);
    assert_eq!(buf, want);
}

#[test]
fn a_console_record_counting_more_bytes_than_it_holds_does_not_decode() {
    let mut buf = Vec::new();
    console().encode(&mut buf);
    // The count follows the tag, unit and console id.
    let at = 1 + 8 + 8;
    buf[at] = CONSOLE_RECORD_BYTES as u8;
    let (full, used) = TraceRecord::decode(&buf).expect("a full record decodes");
    assert_eq!(used, buf.len());
    let TraceRecord::HypervisorConsole { len, .. } = &full else {
        panic!("{full:?}");
    };
    assert_eq!(usize::from(*len), CONSOLE_RECORD_BYTES);
    buf[at] = CONSOLE_RECORD_BYTES as u8 + 1;
    assert_eq!(
        TraceRecord::decode(&buf),
        Err(DecodeError::CountOutOfRange(CONSOLE_RECORD_BYTES as u8 + 1))
    );
}

#[test]
fn a_return_record_counting_more_outputs_than_it_holds_does_not_decode() {
    let mut buf = Vec::new();
    returned().encode(&mut buf);
    // The count follows the tag, unit, status and seven outputs.
    let at = 1 + 8 + 8 + 7 * 8;
    buf[at] = 7;
    let (full, used) = TraceRecord::decode(&buf).expect("seven outputs decode");
    assert_eq!(used, buf.len());
    assert!(
        matches!(full, TraceRecord::HypercallReturned { written: 7, .. }),
        "{full:?}"
    );
    buf[at] = 8;
    assert_eq!(
        TraceRecord::decode(&buf),
        Err(DecodeError::CountOutOfRange(8))
    );
}
