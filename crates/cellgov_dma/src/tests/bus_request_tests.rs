//! A transfer split into its bus requests.

use super::*;
use cellgov_ps3_abi::hw::spu::MfcTagId;

fn range(start: u64, length: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(start), length).expect("range fits")
}

/// A 0x300-byte put from local store 0x1000 to 0x9040, a list element
/// with the stall-and-notify flag, tag 5, under a fence.
fn put() -> DmaRequest {
    DmaRequest::new(
        DmaDirection::Put,
        range(0x1000, 0x300),
        range(0x9040, 0x300),
        UnitId::new(2),
    )
    .expect("equal lengths")
    .with_local_store_source()
    .with_tag_id(MfcTagId::new(5).expect("tag 5"))
    .with_ordering(MfcOrdering::Fence)
    .with_command_word(0x22)
    .with_stall_notify()
}

#[test]
fn parts_end_on_the_destinations_line_boundaries() {
    let parts: Vec<_> = put()
        .bus_requests(128)
        .map(|(end, part)| (end, part.source(), part.destination()))
        .collect();
    let expected = [
        (0x40, 0x1000, 0x9040, 0x40),
        (0xC0, 0x1040, 0x9080, 0x80),
        (0x140, 0x10C0, 0x9100, 0x80),
        (0x1C0, 0x1140, 0x9180, 0x80),
        (0x240, 0x11C0, 0x9200, 0x80),
        (0x2C0, 0x1240, 0x9280, 0x80),
        (0x300, 0x12C0, 0x9300, 0x40),
    ]
    .map(|(end, src, dst, len)| (end, range(src, len), range(dst, len)));
    assert_eq!(parts, expected);
}

#[test]
fn only_the_last_part_keeps_the_slot_and_the_stall() {
    let whole = put();
    let parts: Vec<_> = whole.bus_requests(128).map(|(_, part)| part).collect();
    let (last, rest) = parts.split_last().expect("parts");
    assert_eq!((last.holds_slot(), last.stall_notify()), (true, true));
    assert_eq!(rest.len(), 6);
    for part in rest {
        assert_eq!((part.holds_slot(), part.stall_notify()), (false, false));
    }
    for part in &parts {
        assert_eq!(
            (
                part.issuer(),
                part.tag_id(),
                part.command_word(),
                part.local_store_source()
            ),
            (whole.issuer(), whole.tag_id(), whole.command_word(), true)
        );
    }
}

#[test]
fn only_the_first_part_keeps_the_ordering() {
    let orderings: Vec<_> = put()
        .bus_requests(128)
        .map(|(_, part)| part.ordering())
        .collect();
    let mut expected = vec![MfcOrdering::None; 7];
    expected[0] = MfcOrdering::Fence;
    assert_eq!(orderings, expected);
}

#[test]
fn a_transfer_of_no_bytes_has_no_parts() {
    let empty = DmaRequest::new(
        DmaDirection::Put,
        range(0x1000, 0),
        range(0x9000, 0),
        UnitId::new(0),
    )
    .expect("equal lengths");
    assert_eq!(empty.bus_requests(128).count(), 0);
    assert_eq!(put().bus_requests(0).count(), 0);
}
