use super::*;
use crate::lv1::call::{self, CallSource, GET_VIRTUAL_UART_PARAM};
use crate::lv1::status::{zero_extended, LV1_NO_ENTRY};
use crate::lv1::table::{table_entry, TableEntry};

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 165.
#[test]
fn get_virtual_uart_param_carries_its_header_number_name_and_counts() {
    assert_eq!(GET_VIRTUAL_UART_PARAM, 165);
    let row = call::lookup(GET_VIRTUAL_UART_PARAM).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(
        row,
        Some(("get_virtual_uart_param", 2, 1, CallSource::Linux))
    );
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_165() {
    assert_eq!(VIRTUAL_UART_PARAM_HANDLERS.len(), 1);
    let handler = virtual_uart_param_handler("4.93").expect("the 4.93 handler");
    assert_eq!(
        table_entry("4.93", GET_VIRTUAL_UART_PARAM),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_param_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(virtual_uart_param_handler(firmware), None, "{firmware:?}");
    }
}

/// lv1 builds the port refusal as `(1 << 32) | 0xFFFFFFFA` and the
/// parameter refusal as `(2 << 32) | 0xFFFFFFFA`
/// [OFW-4.93 lv1 a:0x2F3958-0x2F3964] [OFW-4.93 lv1 a:0x2F35CC-0x2F35D8].
#[test]
fn the_port_refusal_marks_r3_and_the_parameter_refusal_marks_r4() {
    assert_eq!(
        GET_VIRTUAL_UART_PARAM_NO_PORT_R3,
        (1 << 32) | zero_extended(LV1_NO_ENTRY)
    );
    assert_eq!(
        GET_VIRTUAL_UART_PARAM_NO_PARAM_R3,
        (2 << 32) | zero_extended(LV1_NO_ENTRY)
    );
}

/// lv1 refuses r3 above 255 [OFW-4.93 lv1 a:0x2F34E0]
/// [OFW-4.93 lv1 a:0x2F34F8] and r4 above 7
/// [OFW-4.93 lv1 a:0x2F35C4-0x2F35C8]. The ids 3 and 5 are
/// `PARAM_RX_BUF_SIZE` and `PARAM_TX_BUF_SIZE` of Linux
/// `drivers/ps3/ps3-vuart.c`.
#[test]
fn the_port_bound_is_255_the_parameter_bound_is_7_and_the_buffer_size_ids_are_3_and_5() {
    assert_eq!(VIRTUAL_UART_PORT_MAX, 255);
    assert_eq!(VIRTUAL_UART_PARAM_MAX, 7);
    assert_eq!(VIRTUAL_UART_PARAM_RX_BUFFER_SIZE, 3);
    assert_eq!(VIRTUAL_UART_PARAM_TX_BUFFER_SIZE, 5);
}

#[test]
fn the_port_rows_ascend_by_firmware_and_port_with_ports_in_bounds() {
    assert_eq!(GAME_OS_VIRTUAL_UART_PORTS.len(), 3);
    for pair in GAME_OS_VIRTUAL_UART_PORTS.windows(2) {
        assert!(
            (pair[0].firmware, pair[0].port) < (pair[1].firmware, pair[1].port),
            "{pair:?}"
        );
    }
    for row in GAME_OS_VIRTUAL_UART_PORTS {
        assert!(row.port <= VIRTUAL_UART_PORT_MAX, "{row:?}");
        assert!(
            virtual_uart_param_handler(row.firmware).is_some(),
            "{row:?}"
        );
    }
}

#[test]
fn no_port_row_exists_for_another_port_or_firmware() {
    for (firmware, port) in [("4.93", 1), ("4.93", 11), ("4.93", 256), ("4.92", 0)] {
        assert_eq!(
            game_os_virtual_uart_port(firmware, port),
            None,
            "{firmware} {port}"
        );
    }
}

/// The 4.93 kernel's open function sends 165 for parameter 3 at its first
/// `sc` and for parameter 5 at its second
/// [OFW-4.93 lv2_kernel a:0x8000000000017714-0x8000000000017750].
#[test]
fn the_4_93_param_calls_are_the_open_function_s_two_sc_addresses() {
    let rows: Vec<(&str, u64, u64)> = GAME_OS_VIRTUAL_UART_PARAM_CALLS
        .iter()
        .map(|call| (call.firmware, call.sc, call.param))
        .collect();
    assert_eq!(
        rows,
        [
            ("4.93", 0x8000_0000_0001_7730, 3),
            ("4.93", 0x8000_0000_0001_7750, 5),
        ]
    );
}

#[test]
fn a_param_call_matches_only_its_own_firmware_sc_and_parameter() {
    let (rx, tx) = (0x8000_0000_0001_7730, 0x8000_0000_0001_7750);
    assert!(is_game_os_virtual_uart_param_call("4.93", rx, 3));
    assert!(is_game_os_virtual_uart_param_call("4.93", tx, 5));
    for (firmware, sc, param) in [
        ("4.93", rx, 5),
        ("4.93", tx, 3),
        ("4.93", rx + 4, 3),
        ("4.93", 0, 5),
        ("4.92", rx, 3),
    ] {
        assert!(
            !is_game_os_virtual_uart_param_call(firmware, sc, param),
            "{firmware} {sc:#x} {param}"
        );
    }
}
