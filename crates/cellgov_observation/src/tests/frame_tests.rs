use super::*;

fn region(name: &str, offset: u64, size: u64, guest_addr: u64) -> FrameRegion {
    FrameRegion {
        name: name.to_string(),
        offset,
        size,
        guest_addr,
    }
}

#[test]
fn regions_slice_at_their_offsets_inside_surrounding_log_text() {
    let mut log = b"boot noise\n".to_vec();
    log.extend(b"CGOV\x00\x00\x00\x06\x01\x02\x03\x04\x05\x06\ntrailing\n");
    let got =
        parse_frame(&log, &[region("a", 0, 2, 0x100), region("b", 4, 2, 0x200)]).expect("parses");
    assert_eq!(
        got.iter()
            .map(|r| (r.name.as_str(), r.addr, r.data.clone()))
            .collect::<Vec<_>>(),
        [("a", 0x100, vec![1, 2]), ("b", 0x200, vec![5, 6])]
    );
}

#[test]
fn every_malformed_input_is_refused_by_name() {
    let one = [region("a", 0, 4, 0)];
    assert_eq!(
        parse_frame(b"nothing", &one),
        Err(FrameError::MagicNotFound)
    );
    assert_eq!(
        parse_frame(b"CGOV\x00\x00", &one),
        Err(FrameError::PayloadTooSmall {
            expected: 8,
            actual: 6
        })
    );
    assert_eq!(
        parse_frame(b"CGOV\x00\x00\x00\x09\x01", &one),
        Err(FrameError::PayloadTooSmall {
            expected: 9,
            actual: 1
        })
    );
    assert_eq!(
        parse_frame(b"CGOV\x00\x00\x00\x02\x01\x02", &one),
        Err(FrameError::PayloadTooSmall {
            expected: 4,
            actual: 2
        })
    );
    assert_eq!(
        parse_frame(
            b"CGOV\x00\x00\x00\x04\x01\x02\x03\x04CGOV\x00\x00\x00\x00",
            &one
        ),
        Err(FrameError::Ambiguous {
            first: 0,
            second: 12
        })
    );
    assert_eq!(
        parse_frame(
            b"CGOV\x00\x00\x00\x04\x01\x02\x03\x04",
            &[region("w", u64::MAX, 2, 0)]
        ),
        Err(FrameError::OffsetOverflow {
            region_name: "w".to_string(),
            offset: u64::MAX,
            size: 2
        })
    );
}

#[test]
fn magic_bytes_inside_the_payload_are_region_data() {
    let got = parse_frame(b"CGOV\x00\x00\x00\x04CGOV", &[region("m", 0, 4, 0)]).expect("parses");
    assert_eq!(got[0].data, b"CGOV");
}
