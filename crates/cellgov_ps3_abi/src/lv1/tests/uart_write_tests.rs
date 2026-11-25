use super::*;
use crate::lv1::call::WRITE_VIRTUAL_UART;
use crate::lv1::status::{zero_extended, LV1_NO_ENTRY, LV1_TYPE_MISMATCH, LV1_WRONG_STATE};
use crate::lv1::table::{table_entry, TableEntry};

/// The header that the 4.93 kernel sends first on port 10: request count
/// 0, service id 0x19000, 0x38 request bytes and 0x38 reply bytes. The
/// client starts its request count at 0
/// [OFW-4.93 lv2_kernel a:0x8000000000212408]. The
/// header function puts the count, the service id and the two sizes in
/// the words at +0, +4, +8 and +12
/// [OFW-4.93 lv2_kernel a:0x80000000002121f4-0x80000000002121fc]
/// [OFW-4.93 lv2_kernel a:0x800000000021221c-0x8000000000212230]. The
/// first request's class gives both sizes as 0x38
/// [OFW-4.93 lv2_kernel a:0x8000000000229c34], and its constructor gives
/// the service id
/// [OFW-4.93 lv2_kernel a:0x8000000000229938-0x8000000000229950].
const KERNEL_HEADER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x90, 0x00, 0x00, 0x00, 0x00, 0x38, 0x00, 0x00, 0x00, 0x38,
];

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_163() {
    assert_eq!(VIRTUAL_UART_WRITE_HANDLERS.len(), 1);
    let handler = virtual_uart_write_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x2F2AF4);
    assert_eq!(
        table_entry("4.93", WRITE_VIRTUAL_UART),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_write_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(virtual_uart_write_handler(firmware), None, "{firmware:?}");
    }
}

/// lv1 builds each refusal as `(mark << 32) | status`
/// [OFW-4.93 lv1 a:0x2F2EC8-0x2F2ED4] [OFW-4.93 lv1 a:0x2F2EF0-0x2F2EFC]
/// [OFW-4.93 lv1 a:0x2F2F10-0x2F2F1C] [OFW-4.93 lv1 a:0x2F2EDC-0x2F2EE8].
#[test]
fn the_port_refusals_mark_r3_and_the_region_refusals_mark_r4() {
    assert_eq!(
        WRITE_VIRTUAL_UART_NO_PORT_R3,
        (1 << 32) | zero_extended(LV1_NO_ENTRY)
    );
    assert_eq!(
        WRITE_VIRTUAL_UART_NO_REGION_R3,
        (2 << 32) | zero_extended(LV1_NO_ENTRY)
    );
    assert_eq!(
        WRITE_VIRTUAL_UART_REGION_R3,
        (2 << 32) | zero_extended(LV1_TYPE_MISMATCH)
    );
    assert_eq!(
        WRITE_VIRTUAL_UART_NO_PEER_R3,
        (1 << 32) | zero_extended(LV1_WRONG_STATE)
    );
}

/// The handler builds the bound as `clrldi r0, -1, 2`
/// [OFW-4.93 lv1 a:0x2F2B98-0x2F2B9C].
#[test]
fn the_write_buffer_address_bound_is_2_to_the_62_less_1() {
    assert_eq!(VIRTUAL_UART_WRITE_ADDRESS_MAX, (1u64 << 62) - 1);
}

/// The port function reads the header with `li r5, 16`
/// [OFW-4.93 lv1 a:0x5BD8BC].
#[test]
fn the_dispatcher_header_is_16_bytes() {
    assert_eq!(DISPATCHER_HEADER_BYTES, 16);
}

/// The port function loads the service id with `lwz r0, 172(r1)` and the
/// request byte count with `lwz r5, 176(r1)`, from a header that it reads
/// to r1 + 168 [OFW-4.93 lv1 a:0x5BD8B8] [OFW-4.93 lv1 a:0x5BD910]
/// [OFW-4.93 lv1 a:0x5BD934].
#[test]
fn the_header_gives_the_service_id_at_4_and_the_request_bytes_at_8() {
    assert_eq!(dispatcher_header_service(&KERNEL_HEADER), 0x19000);
    assert_eq!(dispatcher_header_request_bytes(&KERNEL_HEADER), 0x38);
    let mut header = [0u8; 16];
    header[4..8].copy_from_slice(&0x0102_0304u32.to_be_bytes());
    header[8..12].copy_from_slice(&0x0506_0708u32.to_be_bytes());
    assert_eq!(dispatcher_header_service(&header), 0x0102_0304);
    assert_eq!(dispatcher_header_request_bytes(&header), 0x0506_0708);
}

/// The constructor builds the ids with `lis r11, 0x1` / `ori r11, r11,
/// 0x9002` and `lis r10, 0x1` / `ori r10, r10, 0x9000`
/// [OFW-4.93 lv2_kernel a:0x8000000000229938-0x8000000000229950].
#[test]
fn the_kernel_s_first_request_is_request_0x19002_of_service_0x19000() {
    assert_eq!(GAME_OS_FIRST_DISPATCHER_SERVICE, 0x19000);
    assert_eq!(GAME_OS_FIRST_DISPATCHER_REQUEST, 0x19002);
    assert_eq!(
        dispatcher_header_service(&KERNEL_HEADER),
        GAME_OS_FIRST_DISPATCHER_SERVICE
    );
}
