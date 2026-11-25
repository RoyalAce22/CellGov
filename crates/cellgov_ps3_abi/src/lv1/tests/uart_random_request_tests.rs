use super::*;

#[test]
fn request_0x5003_of_service_0x5000_has_random_reply_data_at_4_93_with_0x40_bytes() {
    let row = game_os_random_dispatcher_request("4.93", 0x5000, 0x5003)
        .expect("an image read shows random reply data");
    assert_eq!(
        *row,
        GameOsRandomDispatcherRequest {
            firmware: "4.93",
            service: 0x5000,
            request: 0x5003,
            bytes: 0x40,
        }
    );
}

#[test]
fn no_other_firmware_service_or_request_has_random_reply_data() {
    assert_eq!(GAME_OS_RANDOM_DISPATCHER_REQUESTS.len(), 1);
    for (firmware, service, request) in [
        ("4.92", 0x5000, 0x5003),
        ("4.93", 0x19000, 0x5003),
        ("4.93", 0x5000, 0x5002),
    ] {
        assert_eq!(
            game_os_random_dispatcher_request(firmware, service, request),
            None,
            "{firmware} {service:#x} {request:#x}"
        );
    }
}

#[test]
fn the_random_data_end_at_the_last_byte_of_the_reply() {
    let row = &GAME_OS_RANDOM_DISPATCHER_REQUESTS[0];
    assert_eq!(
        DISPATCHER_REPLY_DATA_OFFSET + DISPATCHER_RANDOM_DATA_BYTES,
        row.bytes as usize
    );
}
