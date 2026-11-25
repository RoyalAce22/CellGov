use super::*;

/// The size of port 10's receive buffer in the capture
/// `lv2_vuart_port_params`.
const PORT_10_SIZE: u64 = 0x1000;

fn traffic(buffer_count: u64, peer_count: Option<u64>) -> VirtualUartTraffic {
    VirtualUartTraffic {
        buffer_size: PORT_10_SIZE,
        buffer_count,
        peer_count,
    }
}

/// The kernel's open sets each trigger to the buffer size less 1
/// [OFW-4.93 lv2_kernel a:0x80000000000177bc-0x80000000000177ec].
#[test]
fn an_empty_port_with_an_empty_live_peer_after_the_kernel_s_open_has_the_tx_and_connected_bits() {
    let trigger = PORT_10_SIZE - 1;
    assert_eq!(
        virtual_uart_status(traffic(0, Some(0)), trigger, trigger),
        VIRTUAL_UART_STATUS_TX | VIRTUAL_UART_STATUS_CONNECTED
    );
}

/// lv1 tests the peer's count with `cmpld` and `ble`
/// [OFW-4.93 lv1 a:0x2F2200-0x2F2204].
#[test]
fn the_tx_bit_is_set_while_the_peer_s_count_is_not_above_the_transmit_trigger() {
    let rx_trigger = 0;
    for (peer_count, tx_trigger, status) in [
        (0, 0, VIRTUAL_UART_STATUS_TX | VIRTUAL_UART_STATUS_CONNECTED),
        (5, 5, VIRTUAL_UART_STATUS_TX | VIRTUAL_UART_STATUS_CONNECTED),
        (6, 5, VIRTUAL_UART_STATUS_CONNECTED),
        (u64::MAX, u64::MAX - 1, VIRTUAL_UART_STATUS_CONNECTED),
    ] {
        assert_eq!(
            virtual_uart_status(traffic(0, Some(peer_count)), tx_trigger, rx_trigger),
            status,
            "peer {peer_count:#x} trigger {tx_trigger:#x}"
        );
    }
}

/// lv1 tests the buffer size less the count with `cmpld` and `ble`
/// [OFW-4.93 lv1 a:0x2F2208-0x2F2210].
#[test]
fn the_rx_bit_is_set_while_the_free_bytes_are_not_above_the_receive_trigger() {
    let peer = None;
    for (count, rx_trigger, rx) in [
        (0, PORT_10_SIZE - 1, 0),
        (0, PORT_10_SIZE, VIRTUAL_UART_STATUS_RX),
        (1, PORT_10_SIZE - 1, VIRTUAL_UART_STATUS_RX),
        (0x10, 0xFEF, 0),
        (0x11, 0xFEF, VIRTUAL_UART_STATUS_RX),
        (PORT_10_SIZE, 0, VIRTUAL_UART_STATUS_RX),
    ] {
        assert_eq!(
            virtual_uart_status(traffic(count, peer), u64::MAX, rx_trigger),
            VIRTUAL_UART_STATUS_DISCONNECT | rx,
            "count {count:#x} trigger {rx_trigger:#x}"
        );
    }
}

#[test]
fn a_port_with_no_peer_has_the_disconnect_bit_and_never_the_tx_bit() {
    for tx_trigger in [0, u64::MAX] {
        assert_eq!(
            virtual_uart_status(traffic(0, None), tx_trigger, 0),
            VIRTUAL_UART_STATUS_DISCONNECT
        );
    }
}

/// The three mask bits are `INTERRUPT_MASK_TX`, `INTERRUPT_MASK_RX` and
/// `INTERRUPT_MASK_DISCONNECT` of Linux `drivers/ps3/ps3-vuart.c`.
#[test]
fn the_mask_bits_are_the_low_three_bits_and_the_connected_bit_is_above_them() {
    assert_eq!(
        [
            VIRTUAL_UART_STATUS_TX,
            VIRTUAL_UART_STATUS_RX,
            VIRTUAL_UART_STATUS_DISCONNECT
        ],
        [1, 2, 4]
    );
    assert_eq!(
        VIRTUAL_UART_STATUS_TX | VIRTUAL_UART_STATUS_RX | VIRTUAL_UART_STATUS_DISCONNECT,
        VIRTUAL_UART_INTERRUPT_MASK_MAX
    );
    assert_eq!(
        VIRTUAL_UART_STATUS_CONNECTED & VIRTUAL_UART_INTERRUPT_MASK_MAX,
        0
    );
}
