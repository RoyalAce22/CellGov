use super::*;

/// A header with tag 0 for service 0x19000, with `request_bytes` request
/// bytes and as many reply bytes.
fn header(request_bytes: u32) -> [u8; 16] {
    let mut header = [0u8; 16];
    header[4..8].copy_from_slice(&0x19000u32.to_be_bytes());
    header[8..12].copy_from_slice(&request_bytes.to_be_bytes());
    header[12..16].copy_from_slice(&request_bytes.to_be_bytes());
    header
}

#[test]
fn an_empty_request_has_no_header_service_or_doubleword_and_is_not_complete() {
    let empty = PendingRequest::empty(10);
    assert_eq!((empty.port, empty.sent), (10, 0));
    assert_eq!(empty.service(), None);
    assert_eq!(empty.first_doubleword(), None);
    assert!(!empty.is_complete());
}

#[test]
fn the_bytes_of_each_write_follow_the_bytes_sent_before() {
    let h = header(0x38);
    let request = PendingRequest::empty(10).with(&h[..5]).with(&h[5..]);
    assert_eq!(request.sent, 16);
    assert_eq!(request.prefix[..16], h);
    assert_eq!(request.prefix[16..], [0; 8]);
    assert_eq!(request.service(), Some(0x19000));
}

#[test]
fn the_service_is_none_until_the_last_header_byte() {
    let h = header(0x38);
    assert_eq!(PendingRequest::empty(10).with(&h[..15]).service(), None);
    assert_eq!(PendingRequest::empty(10).with(&h).service(), Some(0x19000));
}

#[test]
fn bytes_past_the_prefix_count_toward_sent_and_keep_the_prefix() {
    let h = header(0x100);
    let body: Vec<u8> = (1..=40).collect();
    let request = PendingRequest::empty(10).with(&h).with(&body);
    assert_eq!(request.sent, 56);
    assert_eq!(request.prefix[16..], body[..8]);
    assert_eq!(
        request.first_doubleword(),
        Some(u64::from_be_bytes([1, 2, 3, 4, 5, 6, 7, 8]))
    );
}

#[test]
fn the_first_doubleword_needs_8_request_bytes_sent_and_counted() {
    let partial = PendingRequest::empty(10).with(&header(0x38)).with(&[9; 7]);
    assert_eq!(partial.first_doubleword(), None);
    let sent = partial.with(&[9]);
    assert_eq!(sent.sent, REQUEST_PREFIX_BYTES as u64);
    assert_eq!(sent.first_doubleword(), Some(0x0909_0909_0909_0909));
    let short = PendingRequest::empty(10).with(&header(4)).with(&[9; 8]);
    assert_eq!(short.sent, REQUEST_PREFIX_BYTES as u64);
    assert_eq!(short.first_doubleword(), None);
    let counted = PendingRequest::empty(10).with(&header(8)).with(&[9; 8]);
    assert_eq!(counted.first_doubleword(), Some(0x0909_0909_0909_0909));
}

#[test]
fn a_request_is_complete_at_the_header_plus_its_request_bytes() {
    let h = header(0x38);
    let base = PendingRequest::empty(10).with(&h);
    assert!(!base.with(&[0; 0x37]).is_complete());
    assert!(base.with(&[0; 0x38]).is_complete());
    assert!(base.with(&[0; 0x39]).is_complete());
    assert!(PendingRequest::empty(10).with(&header(0)).is_complete());
}
