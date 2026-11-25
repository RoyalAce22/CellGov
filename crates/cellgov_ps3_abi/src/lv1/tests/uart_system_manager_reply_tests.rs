use super::*;

#[test]
fn the_reply_header_holds_version_1_size_16_the_count_the_service_halfword_and_the_tag() {
    assert_eq!(
        system_manager_reply_header(0x0B, 16, 0x0A0B_0C0D),
        [
            0x01, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x0B, 0x00, 0x00, 0x0A, 0x0B,
            0x0C, 0x0D,
        ]
    );
}

#[test]
fn the_reply_header_reads_back_through_the_request_header_accessors() {
    let header = system_manager_reply_header(0x0B, 16, 7);
    assert_eq!(system_manager_header_payload_bytes(&header), 16);
    assert_eq!(system_manager_header_service(&header), 0x000B);
    assert_eq!(system_manager_header_tag(&header), 7);
}

#[test]
fn service_9_with_payload_byte_1_2_at_4_93_gets_the_16_byte_reply_of_service_0x0b() {
    let reply = game_os_system_manager_reply("4.93", GAME_OS_FIRST_SYSTEM_MANAGER_SERVICE, 2)
        .expect("a source gives the reply");
    assert_eq!(reply.reply_service, 0x0B);
    assert_eq!(
        reply.payload,
        [2, 1, 0, 0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 7]
    );
}

#[test]
fn a_service_id_whose_low_byte_is_9_takes_the_reply_of_handler_9() {
    assert_eq!(
        game_os_system_manager_reply("4.93", 0x0109, 2),
        game_os_system_manager_reply("4.93", 9, 2)
    );
    assert!(game_os_system_manager_reply("4.93", 0x0109, 2).is_some());
}

#[test]
fn no_source_gives_another_payload_byte_1_another_handler_or_another_firmware() {
    for (firmware, service, selector) in [
        ("4.93", 9, 1),
        ("4.93", 9, 3),
        ("4.93", 9, 0),
        ("4.93", 0x0A, 2),
        ("4.92", 9, 2),
    ] {
        assert_eq!(
            game_os_system_manager_reply(firmware, service, selector),
            None,
            "{firmware} {service:#x} {selector}"
        );
    }
}
