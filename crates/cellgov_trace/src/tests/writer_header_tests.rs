//! Writer rules for the identity header: it bypasses the level filter,
//! and no other record enters through that path.

use super::*;
use crate::hash::StateHash;
use crate::record::{HashCheckpointKind, TraceRecord};
use cellgov_event::UnitId;
use cellgov_time::{Budget, Epoch, GuestTicks};

fn scheduled() -> TraceRecord {
    TraceRecord::UnitScheduled {
        unit: UnitId::new(0),
        granted_budget: Budget::new(1),
        time: GuestTicks::ZERO,
        epoch: Epoch::ZERO,
    }
}

fn hash_checkpoint() -> TraceRecord {
    TraceRecord::StateHashCheckpoint {
        kind: HashCheckpointKind::CommittedMemory,
        hash: StateHash::new(42),
    }
}

fn header() -> TraceRecord {
    TraceRecord::RunIdentity {
        format_version: crate::record::TRACE_FORMAT_VERSION,
        firmware: 7,
        game: 9,
        overrides: 11,
    }
}

#[test]
fn header_is_written_through_a_filter_that_drops_its_level() {
    let mut w = TraceWriter::with_levels(&[TraceLevel::Hashes]);
    assert!(w.record_header(&header()));
    assert!(!w.record(&scheduled()));
    assert_eq!(w.record_count(), 1);
    assert_eq!(
        crate::TraceReader::new(w.bytes())
            .next()
            .expect("header decodes")
            .expect("header decodes"),
        header()
    );
}

#[test]
fn a_header_offered_after_a_record_is_refused() {
    let mut w = TraceWriter::new();
    assert!(w.record(&scheduled()));
    let bytes_before = w.byte_len();
    assert!(!w.record_header(&header()));
    assert_eq!(w.byte_len(), bytes_before);
    assert_eq!(w.record_count(), 1);
}

#[test]
fn a_second_header_is_refused() {
    let mut w = TraceWriter::new();
    assert!(w.record_header(&header()));
    assert!(!w.record_header(&header()));
    assert_eq!(w.record_count(), 1);
    assert_eq!(Some(w.byte_len()), TraceRecord::encoded_len(header().tag()));
}

#[test]
fn record_header_refuses_a_record_that_is_not_the_header() {
    let mut w = TraceWriter::with_levels(&[]);
    assert!(!w.record_header(&hash_checkpoint()));
    assert_eq!(w.record_count(), 0);
    assert_eq!(w.byte_len(), 0);
}

#[test]
fn the_header_cannot_enter_the_stream_through_the_filtered_path() {
    let mut w = TraceWriter::new();
    assert!(!w.record(&header()));
    assert_eq!(w.record_count(), 0);
    assert_eq!(w.byte_len(), 0);
}
