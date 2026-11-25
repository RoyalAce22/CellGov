use super::*;

/// The dispatcher manager's port [OFW-4.93 lv1 a:0x5B7E7C-0x5B7E80].
const PORT: u64 = 10;

/// The counts of the kernel's class for request 0x19004 at 4.93
/// [OFW-4.93 lv2_kernel a:0x8000000000229f34].
const IDENTITY_BYTES: usize = 0x30;

const TAG: u32 = 3;

/// The offset of the reply bytes after the reply header.
const BODY: usize = DISPATCHER_HEADER_BYTES as usize;

/// A complete request 0x19004 of service 0x19000 with 0x30 request and
/// reply bytes, and its request bytes.
///
/// Each request byte past the two ids is 0xA5, so a reply byte that the
/// service writes differs from the byte that it echoes.
fn identity_request() -> (PendingRequest, Vec<u8>) {
    let mut header = [0u8; 16];
    header[..4].copy_from_slice(&TAG.to_be_bytes());
    header[4..8].copy_from_slice(&0x19000u32.to_be_bytes());
    header[8..12].copy_from_slice(&(IDENTITY_BYTES as u32).to_be_bytes());
    header[12..].copy_from_slice(&(IDENTITY_BYTES as u32).to_be_bytes());
    let mut body = vec![0xA5u8; IDENTITY_BYTES];
    body[..8].copy_from_slice(&0x19004u64.to_be_bytes());
    body[8..16].copy_from_slice(&0x19000u64.to_be_bytes());
    (PendingRequest::empty(PORT).with(&header), body)
}

fn data_of(reply: &[u8]) -> &[u8] {
    &reply[BODY + DISPATCHER_REPLY_DATA_OFFSET..BODY + IDENTITY_BYTES]
}

#[test]
fn request_0x19004_gets_status_0_and_the_data_bytes_of_the_synthetic_identity() {
    let (pending, body) = identity_request();
    let mut expected = body.clone();
    expected[DISPATCHER_REPLY_STATUS_OFFSET..DISPATCHER_REPLY_STATUS_OFFSET + 4]
        .copy_from_slice(&0u32.to_be_bytes());
    expected[DISPATCHER_REPLY_DATA_OFFSET..]
        .copy_from_slice(&SYNTHETIC_CONSOLE_IDENTITY.dispatcher_data());
    let expected = [
        &dispatcher_reply_header(TAG, IDENTITY_BYTES as u32)[..],
        &expected,
    ]
    .concat();
    assert_eq!(dispatcher_reply("4.93", &pending, &body), Ok(expected));
}

#[test]
fn a_changed_identity_changes_only_the_answered_data_bytes() {
    let other = ConsoleIdentity {
        product_code: 0x85,
        product_sub_code: 3,
    };
    assert_ne!(other, SYNTHETIC_CONSOLE_IDENTITY);
    let (pending, body) = identity_request();
    let synthetic =
        dispatcher_reply_with("4.93", &SYNTHETIC_CONSOLE_IDENTITY, &pending, &body).unwrap();
    let changed = dispatcher_reply_with("4.93", &other, &pending, &body).unwrap();
    assert_eq!(
        data_of(&synthetic),
        SYNTHETIC_CONSOLE_IDENTITY.dispatcher_data()
    );
    assert_eq!(data_of(&changed), other.dispatcher_data());
    assert_ne!(data_of(&synthetic), data_of(&changed));
    let data = BODY + DISPATCHER_REPLY_DATA_OFFSET;
    assert_eq!(synthetic.len(), changed.len());
    assert_eq!(synthetic[..data], changed[..data]);
}
