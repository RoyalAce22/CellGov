//! Wire shape of the identity header record.

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
fn run_identity_encode_decode_roundtrip() {
    roundtrip(TraceRecord::RunIdentity {
        format_version: TRACE_FORMAT_VERSION,
        firmware: 0xdead_beef_0000_0001,
        game: 0x0000_0002_cafe_f00d,
        overrides: 0x0003_0000_0000_b00d,
    });
}

#[test]
fn run_identity_absent_halves_roundtrip_as_zero_fingerprints() {
    roundtrip(TraceRecord::RunIdentity {
        format_version: TRACE_FORMAT_VERSION,
        firmware: 0,
        game: 0,
        overrides: 0,
    });
}

#[test]
fn run_identity_tag_is_0x0d() {
    let r = TraceRecord::RunIdentity {
        format_version: 0,
        firmware: 0,
        game: 0,
        overrides: 0,
    };
    let mut buf = Vec::new();
    r.encode(&mut buf);
    assert_eq!(buf[0], 0x0d);
    assert_eq!(
        buf.len(),
        29,
        "documented wire size: 1 tag + format_version as u32 + three u64 fingerprints"
    );
    assert_eq!(r.level(), TraceLevel::Scheduling);
}
