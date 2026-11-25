use super::*;

#[test]
fn service_9_with_payload_byte_1_1_at_4_93_gets_the_8_byte_reply_of_service_0x0a_and_no_stored_bytes(
) {
    let reply = game_os_system_manager_reply("4.93", GAME_OS_FIRST_SYSTEM_MANAGER_SERVICE, 1)
        .expect("a source gives the reply");
    assert_eq!(reply.reply_service, 0x0A);
    assert_eq!(reply.payload, [1, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(reply.trailing, [0u8; 0]);
    assert_eq!(reply.header_count(), 8);
}

#[test]
fn each_0x0a_reply_echoes_its_selector_and_counts_its_trailing_bytes_at_payload_byte_2() {
    let rows: Vec<_> = GAME_OS_SYSTEM_MANAGER_REPLIES
        .iter()
        .filter(|row| row.reply_service == 0x0A)
        .collect();
    assert_eq!(rows.len(), 1);
    for row in rows {
        assert_eq!(row.payload.len(), 8, "{row:?}");
        assert_eq!(row.payload[0], row.selector, "{row:?}");
        let count = u16::from_be_bytes([row.payload[2], row.payload[3]]);
        assert_eq!(usize::from(count), row.trailing.len(), "{row:?}");
    }
}

/// A reply of service id 0x0A with 3 stored bytes.
const WITH_STORED_BYTES: GameOsSystemManagerReply = GameOsSystemManagerReply {
    firmware: "4.93",
    handler: 9,
    selector: 1,
    reply_service: 0x0A,
    payload: &[1, 0, 0, 3, 0, 0, 0, 0],
    trailing: &[0xAA, 0xBB, 0xCC],
};

#[test]
fn the_header_count_adds_the_trailing_bytes_to_the_payload_bytes() {
    assert_eq!(WITH_STORED_BYTES.header_count(), 11);
}

#[test]
fn the_written_bytes_are_the_header_with_the_tag_then_the_payload_then_the_trailing_bytes() {
    assert_eq!(
        WITH_STORED_BYTES.written_bytes(0x0102_0304),
        Some(vec![
            0x01, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0B, 0x00, 0x0A, 0x00, 0x00, 0x01, 0x02,
            0x03, 0x04, 0x01, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0xAA, 0xBB, 0xCC,
        ])
    );
}

#[test]
fn no_row_answers_a_request_of_service_0x0a_with_any_payload_byte_1() {
    for selector in 0..=u8::MAX {
        assert_eq!(
            game_os_system_manager_reply("4.93", 0x0A, selector),
            None,
            "{selector}"
        );
    }
}
