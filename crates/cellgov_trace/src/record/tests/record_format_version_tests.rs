//! The format-3 header's layout, and the refusal by name of a header
//! written under another trace format.

use super::codec::*;
use super::error::*;
use super::trace_record::*;
use crate::hash::StateHash;

#[test]
fn the_header_is_tag_version_then_the_firmware_game_and_override_fingerprints() {
    let mut buf = Vec::new();
    TraceRecord::RunIdentity {
        format_version: TRACE_FORMAT_VERSION,
        firmware: 1,
        game: 2,
        overrides: 0x0102_0304_0506_0708,
    }
    .encode(&mut buf);
    let mut want = vec![0x0d, 3, 0, 0, 0];
    want.extend_from_slice(&1u64.to_le_bytes());
    want.extend_from_slice(&2u64.to_le_bytes());
    want.extend_from_slice(&0x0102_0304_0506_0708u64.to_le_bytes());
    assert_eq!(buf, want);
}

/// The format-2 header: tag, version, and two u64 fingerprints.
fn format_2_header() -> Vec<u8> {
    let mut buf = vec![TAG_RUN_IDENTITY];
    write_u32(&mut buf, 2);
    write_u64(&mut buf, 7);
    write_u64(&mut buf, 9);
    buf
}

fn state_hash(step: u64) -> TraceRecord {
    TraceRecord::PpuStateHash {
        step,
        pc: 0x1_0000 + 4 * step,
        hash: StateHash::new(0xa5a5_0000 + step),
    }
}

#[test]
fn a_format_2_header_is_refused_before_it_frames_the_stream() {
    let mut bytes = format_2_header();
    for step in 0..4 {
        state_hash(step).encode(&mut bytes);
    }
    assert_eq!(
        TraceRecord::decode(&bytes),
        Err(DecodeError::UnsupportedFormatVersion(2))
    );
}

#[test]
fn a_lone_format_2_header_is_refused_as_that_format_not_as_truncated() {
    assert_eq!(
        TraceRecord::decode(&format_2_header()),
        Err(DecodeError::UnsupportedFormatVersion(2))
    );
}

#[test]
fn a_header_from_a_later_format_is_refused() {
    let mut bytes = Vec::new();
    TraceRecord::RunIdentity {
        format_version: TRACE_FORMAT_VERSION + 1,
        firmware: 1,
        game: 2,
        overrides: 3,
    }
    .encode(&mut bytes);
    assert_eq!(
        TraceRecord::decode(&bytes),
        Err(DecodeError::UnsupportedFormatVersion(
            TRACE_FORMAT_VERSION + 1
        ))
    );
}

#[test]
fn a_reader_yields_the_refusal_once_and_then_stops() {
    let mut bytes = format_2_header();
    state_hash(0).encode(&mut bytes);
    let mut reader = crate::TraceReader::new(&bytes);
    assert_eq!(
        reader.next(),
        Some(Err(DecodeError::UnsupportedFormatVersion(2)))
    );
    assert_eq!(reader.next(), None);
}

#[test]
fn the_refusal_names_both_formats() {
    let text = DecodeError::UnsupportedFormatVersion(2).to_string();
    assert!(text.contains("trace format 2"), "{text}");
    assert!(
        text.contains(&format!("reads format {TRACE_FORMAT_VERSION}")),
        "{text}"
    );
}
