//! Wire shape of the state-hash scheme record.

use super::codec::*;
use super::trace_record::*;
use crate::level::TraceLevel;

#[test]
fn scheme_record_wire_format_golden() {
    let r = TraceRecord::StateHashScheme {
        ppu: 0x0123_4567_89ab_cdef,
    };
    let mut buf = Vec::new();
    r.encode(&mut buf);
    assert_eq!(
        buf,
        [0x0f, 0xef, 0xcd, 0xab, 0x89, 0x67, 0x45, 0x23, 0x01],
        "documented wire form: tag 0x0f, then the scheme id as 8 LE bytes"
    );
    assert_eq!(buf[0], TAG_STATE_HASH_SCHEME);
    assert_eq!(r.level(), TraceLevel::Hashes);
}

#[test]
fn scheme_record_boundary_values_roundtrip() {
    for ppu in [0, 1, u64::MAX] {
        let r = TraceRecord::StateHashScheme { ppu };
        let mut buf = Vec::new();
        r.encode(&mut buf);
        assert_eq!(TraceRecord::decode(&buf), Ok((r, buf.len())));
    }
}

#[test]
fn a_header_then_a_scheme_record_decode_in_order() {
    let header = TraceRecord::RunIdentity {
        format_version: TRACE_FORMAT_VERSION,
        firmware: 1,
        game: 2,
        overrides: 0,
    };
    let scheme = TraceRecord::StateHashScheme { ppu: 9 };
    let mut buf = Vec::new();
    header.encode(&mut buf);
    scheme.encode(&mut buf);
    let (first, used) = TraceRecord::decode(&buf).expect("header decodes");
    assert_eq!(first, header);
    assert_eq!(TraceRecord::decode(&buf[used..]), Ok((scheme, 9)));
}
