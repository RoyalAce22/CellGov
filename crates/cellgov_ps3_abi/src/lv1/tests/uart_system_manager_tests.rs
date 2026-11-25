use super::*;

/// The header of the 4.93 kernel's first system manager request in
/// CellGov's boot, with the tag 0x0A0B0C0D in place of 0
/// [OFW-4.93 lv2_kernel a:0x80000000002646e0-0x8000000000264718].
const HEADER: [u8; 16] = [
    0x01, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x09, 0x00, 0x00, 0x0A, 0x0B, 0x0C, 0x0D,
];

#[test]
fn the_payload_count_service_id_and_tag_are_at_4_8_and_12() {
    assert_eq!(system_manager_header_payload_bytes(&HEADER), 8);
    assert_eq!(system_manager_header_service(&HEADER), 9);
    assert_eq!(system_manager_header_tag(&HEADER), 0x0A0B_0C0D);
}

#[test]
fn the_service_id_is_the_whole_halfword_at_8() {
    let mut header = HEADER;
    header[8] = 0x01;
    assert_eq!(system_manager_header_service(&header), 0x0109);
}

#[test]
fn the_payload_read_is_none_for_0_the_count_up_to_2048_and_2048_above() {
    let cases = [
        (0, None),
        (1, Some(1)),
        (8, Some(8)),
        (2048, Some(2048)),
        (2049, Some(2048)),
        (0x7FFF_FFFF, Some(2048)),
    ];
    for (count, read) in cases {
        assert_eq!(system_manager_payload_read(count), read, "{count:#x}");
    }
}

#[test]
fn a_payload_count_that_is_negative_as_a_signed_word_reads_no_payload_byte() {
    for count in [0x8000_0000, 0xFFFF_F800, 0xFFFF_FFFF] {
        assert_eq!(system_manager_payload_read(count), Some(0), "{count:#x}");
    }
}

#[test]
fn the_handler_index_is_the_low_byte_up_to_0x3d_and_6_above() {
    let cases = [
        (0x0000, 0),
        (0x0009, 9),
        (0x0109, 9),
        (0xFF3D, 0x3D),
        (0x003E, 6),
        (0x00FF, 6),
    ];
    for (service, index) in cases {
        assert_eq!(
            system_manager_handler_index(service),
            index,
            "{service:#06x}"
        );
    }
}

#[test]
fn the_kernel_s_first_request_takes_entry_9() {
    assert_eq!(
        system_manager_handler_index(GAME_OS_FIRST_SYSTEM_MANAGER_SERVICE),
        9
    );
}
