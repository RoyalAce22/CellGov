use super::*;

/// The dispatcher manager's port [OFW-4.93 lv1 a:0x5B7E7C-0x5B7E80].
const PORT: u64 = 10;

/// A header with the tag `tag` for `service`, with `request` request
/// bytes and `reply` reply bytes [OFW-4.93 lv2_kernel a:0x80000000002121f4].
fn header(tag: u32, service: u32, request: u32, reply: u32) -> [u8; 16] {
    let mut header = [0u8; 16];
    header[..4].copy_from_slice(&tag.to_be_bytes());
    header[4..8].copy_from_slice(&service.to_be_bytes());
    header[8..12].copy_from_slice(&request.to_be_bytes());
    header[12..].copy_from_slice(&reply.to_be_bytes());
    header
}

/// 0x38 request bytes with the request id `id` and service 0x19000 at +0
/// and +8, and the byte value `fill` in each other byte.
fn body(id: u64, fill: u8) -> Vec<u8> {
    let mut body = vec![fill; 0x38];
    body[..8].copy_from_slice(&id.to_be_bytes());
    body[8..16].copy_from_slice(&0x19000u64.to_be_bytes());
    body
}

fn after_header(header: [u8; 16]) -> PendingRequest {
    PendingRequest::empty(PORT).with(&header)
}

fn kernel_reply() -> &'static cellgov_ps3_abi::lv1::uart::GameOsDispatcherReply {
    game_os_dispatcher_reply("4.93", 0x19000, 0x19002).expect("a source gives the reply")
}

/// The reply to `request` under a header with tag 0: the reply header
/// with 0x38 reply bytes, then `request` with the status at +16 and the
/// data bytes at +40.
fn expected_reply(request: &[u8]) -> Vec<u8> {
    let mut expected = vec![0u8; 15];
    expected.push(0x38);
    let mut echoed = request.to_vec();
    echoed[16..20].copy_from_slice(&kernel_reply().status.to_be_bytes());
    echoed[40..56].copy_from_slice(&kernel_reply().data);
    expected.extend_from_slice(&echoed);
    expected
}

#[test]
fn the_reply_is_the_reply_header_then_the_request_bytes_with_the_status_and_data_written() {
    let request = body(0x19002, 0xA5);
    let reply = dispatcher_reply(
        "4.93",
        &after_header(header(0, 0x19000, 0x38, 0x38)),
        &request,
    )
    .expect("a reply");
    assert_eq!(reply.len(), 16 + 0x38);
    assert_eq!(reply, expected_reply(&request));
    assert_eq!(reply[16 + 20..16 + 40], [0xA5; 20]);
}

#[test]
fn the_reply_header_carries_the_request_s_tag() {
    let reply = dispatcher_reply(
        "4.93",
        &after_header(header(0x0102_0304, 0x19000, 0x38, 0x38)),
        &body(0x19002, 0),
    )
    .expect("a reply");
    assert_eq!(
        reply[..16],
        [1, 2, 3, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x38]
    );
}

#[test]
fn the_request_bytes_may_come_in_two_writes_only_while_the_first_ends_within_24_bytes() {
    let request = body(0x19002, 0x11);
    let whole = dispatcher_reply(
        "4.93",
        &after_header(header(0, 0x19000, 0x38, 0x38)),
        &request,
    );
    let split = dispatcher_reply(
        "4.93",
        &after_header(header(0, 0x19000, 0x38, 0x38)).with(&request[..8]),
        &request[8..],
    );
    assert_eq!(whole, Ok(expected_reply(&request)));
    assert_eq!(split, whole);
    let kept_too_few = dispatcher_reply(
        "4.93",
        &after_header(header(0, 0x19000, 0x38, 0x38)).with(&request[..9]),
        &request[9..],
    );
    assert_eq!(kept_too_few, Err(NoReply::Unkept));
}

#[test]
fn bytes_past_the_end_of_the_request_are_a_refusal() {
    let mut bytes = body(0x19002, 0);
    bytes.push(0);
    assert_eq!(
        dispatcher_reply(
            "4.93",
            &after_header(header(0, 0x19000, 0x38, 0x38)),
            &bytes
        ),
        Err(NoReply::Overrun)
    );
}

#[test]
fn bytes_that_do_not_complete_the_request_are_a_refusal() {
    let bytes = body(0x19002, 0);
    assert_eq!(
        dispatcher_reply(
            "4.93",
            &after_header(header(0, 0x19000, 0x38, 0x38)),
            &bytes[..0x37]
        ),
        Err(NoReply::Incomplete)
    );
    assert_eq!(
        dispatcher_reply("4.93", &PendingRequest::empty(PORT), &[0; 15]),
        Err(NoReply::Incomplete)
    );
}

#[test]
fn counts_other_than_the_reply_s_are_a_refusal() {
    for (request, reply) in [(0x38, 0x40), (0x40, 0x38)] {
        let mut bytes = body(0x19002, 0);
        bytes.resize(request as usize, 0);
        assert_eq!(
            dispatcher_reply(
                "4.93",
                &after_header(header(0, 0x19000, request, reply)),
                &bytes
            ),
            Err(NoReply::Unsourced(Unsourced {
                service: 0x19000,
                request: Some(0x19002),
                part: SIZE_PART,
            })),
            "{request:#x} {reply:#x}"
        );
    }
}

#[test]
fn a_request_with_no_source_for_its_reply_gets_the_reason_for_its_service() {
    let cases = [
        ("4.93", 0x19000, 0x19005, SERVICE_REQUEST_PART),
        ("4.92", 0x19000, 0x19002, SERVICE_REQUEST_PART),
        ("4.93", 0x9000, 0x19002, REQUEST_PART),
    ];
    for (firmware, service, id, part) in cases {
        assert_eq!(
            dispatcher_reply(
                firmware,
                &after_header(header(0, service, 0x38, 0x38)),
                &body(id, 0)
            ),
            Err(NoReply::Unsourced(Unsourced {
                service,
                request: Some(id),
                part,
            })),
            "{firmware} {service:#x} {id:#x}"
        );
    }
}
