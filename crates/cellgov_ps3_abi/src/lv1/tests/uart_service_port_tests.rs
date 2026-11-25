use super::*;
use crate::lv1::call::{self, CallSource, WRITE_VIRTUAL_UART};

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 163.
#[test]
fn write_virtual_uart_carries_its_header_number_name_and_counts() {
    assert_eq!(WRITE_VIRTUAL_UART, 163);
    let row = call::lookup(WRITE_VIRTUAL_UART).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(row, Some(("write_virtual_uart", 3, 1, CallSource::Linux)));
}

/// The 4.93 system manager constructs port 2 with `li r4, 2` and a buffer
/// of `li r5, 2048` [OFW-4.93 lv1 a:0x44CF04-0x44CF0C].
#[test]
fn the_4_93_system_manager_port_is_port_2_with_a_2048_byte_buffer() {
    assert_eq!(
        game_os_service_port("4.93", 2),
        Some(&GameOsServicePort {
            firmware: "4.93",
            port: 2,
            service: VirtualUartService::SystemManager,
            buffer_bytes: 2048,
        })
    );
}

/// The secure loader code gives the dispatcher manager the port with
/// `li r5, 10` [OFW-4.93 lv1 a:0x5B7E7C], and the dispatcher manager gives
/// the buffer `li r5, 4096` [OFW-4.93 lv1 a:0x5BF1F8].
#[test]
fn the_4_93_dispatcher_manager_port_is_port_10_with_a_4096_byte_buffer() {
    assert_eq!(
        game_os_service_port("4.93", 10),
        Some(&GameOsServicePort {
            firmware: "4.93",
            port: 10,
            service: VirtualUartService::DispatcherManager,
            buffer_bytes: 4096,
        })
    );
}

/// The capture `lv2_vuart_port_params` read both sizes of each port from
/// the kernel's port record.
#[test]
fn each_service_buffer_is_both_buffer_sizes_of_the_captured_port() {
    assert_eq!(GAME_OS_SERVICE_PORTS.len(), 2);
    for row in GAME_OS_SERVICE_PORTS {
        let captured =
            game_os_virtual_uart_port(row.firmware, row.port).expect("the captured port");
        assert_eq!(
            captured,
            &GameOsVirtualUartPort {
                firmware: row.firmware,
                port: row.port,
                rx_buffer_size: row.buffer_bytes,
                tx_buffer_size: row.buffer_bytes,
            },
            "{}",
            row.port
        );
    }
}

/// No read of the firmware images shows the service that holds the peer
/// of port 0.
#[test]
fn no_service_port_row_exists_for_port_0_or_a_firmware_whose_lv1_is_not_read() {
    for (firmware, port) in [("4.93", 0), ("4.93", 1), ("4.92", 2), ("3.70", 10), ("", 2)] {
        assert_eq!(
            game_os_service_port(firmware, port),
            None,
            "{firmware:?} {port}"
        );
    }
}
