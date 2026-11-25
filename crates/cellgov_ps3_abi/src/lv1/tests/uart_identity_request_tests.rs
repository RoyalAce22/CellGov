use super::*;

#[test]
fn request_0x19004_of_service_0x19000_has_identity_reply_data_at_4_93_with_0x30_bytes() {
    let row = game_os_identity_dispatcher_request("4.93", 0x19000, 0x19004)
        .expect("an image read shows identity reply data");
    assert_eq!(
        *row,
        GameOsIdentityDispatcherRequest {
            firmware: "4.93",
            service: 0x19000,
            request: 0x19004,
            bytes: 0x30,
            status: 0,
        }
    );
}

#[test]
fn no_other_firmware_service_or_request_has_identity_reply_data() {
    assert_eq!(GAME_OS_IDENTITY_DISPATCHER_REQUESTS.len(), 1);
    for (firmware, service, request) in [
        ("4.92", 0x19000, 0x19004),
        ("4.93", 0x5000, 0x19004),
        ("4.93", 0x19000, 0x19002),
        ("4.93", 0x19000, 0x19005),
    ] {
        assert_eq!(
            game_os_identity_dispatcher_request(firmware, service, request),
            None,
            "{firmware} {service:#x} {request:#x}"
        );
    }
}

#[test]
fn the_identity_data_end_at_the_last_byte_of_the_reply() {
    let row = &GAME_OS_IDENTITY_DISPATCHER_REQUESTS[0];
    assert_eq!(
        DISPATCHER_REPLY_DATA_OFFSET + DISPATCHER_IDENTITY_DATA_BYTES,
        row.bytes as usize
    );
}
