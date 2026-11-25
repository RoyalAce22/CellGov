use super::*;

/// The header that the 4.93 kernel sends first in CellGov's boot, with
/// the tag 7 in place of 0 and the reply count 0x20 in place of 0x38
/// [OFW-4.93 lv2_kernel a:0x80000000002121f4].
const HEADER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x07, 0x00, 0x01, 0x90, 0x00, 0x00, 0x00, 0x00, 0x38, 0x00, 0x00, 0x00, 0x20,
];

#[test]
fn the_tag_and_the_reply_count_are_the_words_at_0_and_12() {
    assert_eq!(dispatcher_header_tag(&HEADER), 7);
    assert_eq!(dispatcher_header_reply_bytes(&HEADER), 0x20);
    assert_eq!(dispatcher_header_request_bytes(&HEADER), 0x38);
}

#[test]
fn the_reply_header_holds_the_tag_two_zero_words_and_the_reply_count() {
    assert_eq!(
        dispatcher_reply_header(0x0102_0304, 0x38),
        [1, 2, 3, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x38]
    );
}

#[test]
fn the_kernel_s_first_request_has_a_reply_at_4_93_only() {
    let reply = game_os_dispatcher_reply(
        "4.93",
        GAME_OS_FIRST_DISPATCHER_SERVICE,
        GAME_OS_FIRST_DISPATCHER_REQUEST,
    )
    .expect("a source gives the reply");
    assert_eq!(reply.bytes, 0x38);
    assert_eq!(
        game_os_dispatcher_reply(
            "4.92",
            GAME_OS_FIRST_DISPATCHER_SERVICE,
            GAME_OS_FIRST_DISPATCHER_REQUEST,
        ),
        None
    );
    assert_eq!(
        game_os_dispatcher_reply("4.93", GAME_OS_FIRST_DISPATCHER_SERVICE, 0x19003),
        None
    );
    assert_eq!(
        game_os_dispatcher_reply("4.93", 0x17000, GAME_OS_FIRST_DISPATCHER_REQUEST),
        None
    );
}

/// lv1's reply class writes the status word at +16
/// [OFW-4.93 lv1 a:0x58D684-0x58D6A8] and the 16 data bytes at +40
/// [OFW-4.93 lv1 a:0x56D234-0x56D248]. The kernel reads the status at +16
/// [OFW-4.93 lv2_kernel a:0x8000000000212d44-0x8000000000212d6c] and the
/// 16 data bytes at +40
/// [OFW-4.93 lv2_kernel a:0x8000000000229ab0-0x8000000000229b38].
#[test]
fn the_status_word_is_at_16_and_the_16_data_bytes_are_at_40() {
    assert_eq!(DISPATCHER_REPLY_STATUS_OFFSET, 16);
    assert_eq!(DISPATCHER_REPLY_DATA_OFFSET, 40);
    assert_eq!(DISPATCHER_REPLY_DATA_BYTES, 16);
}

#[test]
fn the_status_word_and_the_data_bytes_fit_in_each_reply() {
    const { assert!(DISPATCHER_REPLY_STATUS_OFFSET + 4 <= DISPATCHER_REPLY_DATA_OFFSET) };
    assert_eq!(GAME_OS_DISPATCHER_REPLIES.len(), 1);
    for reply in GAME_OS_DISPATCHER_REPLIES {
        assert!(
            DISPATCHER_REPLY_DATA_OFFSET + DISPATCHER_REPLY_DATA_BYTES <= reply.bytes as usize,
            "{reply:?}"
        );
    }
}
