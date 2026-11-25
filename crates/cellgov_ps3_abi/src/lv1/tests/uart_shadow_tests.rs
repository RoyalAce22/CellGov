use super::*;

/// lv1's status function takes doubleword port / 64 and bit
/// 63 - port % 64 [OFW-4.93 lv1 a:0x2F2214-0x2F2248].
#[test]
fn each_port_names_its_shadow_doubleword_and_bit() {
    assert_eq!(virtual_uart_shadow_bit(0), (0, 1 << 63));
    assert_eq!(virtual_uart_shadow_bit(10), (0, 0x0020_0000_0000_0000));
    assert_eq!(virtual_uart_shadow_bit(63), (0, 1));
    assert_eq!(virtual_uart_shadow_bit(64), (1, 1 << 63));
    assert_eq!(virtual_uart_shadow_bit(u8::MAX), (3, 1));
}

/// Each port's doubleword is inside the 32-byte bitmap.
#[test]
fn the_last_port_names_the_last_bitmap_doubleword() {
    let (doubleword, _) = virtual_uart_shadow_bit(VIRTUAL_UART_PORT_MAX as u8);
    assert_eq!((doubleword + 1) * 8, VIRTUAL_UART_IRQ_BITMAP_BYTES);
}
