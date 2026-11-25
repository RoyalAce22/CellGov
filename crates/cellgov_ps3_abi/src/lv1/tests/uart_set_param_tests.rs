use super::*;
use crate::lv1::call::{self, CallSource, SET_VIRTUAL_UART_PARAM};
use crate::lv1::status::{zero_extended, LV1_ILLEGAL_PARAMETER_VALUE, LV1_NO_ENTRY};
use crate::lv1::table::{table_entry, TableEntry};

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 164.
#[test]
fn set_virtual_uart_param_carries_its_header_number_name_and_counts() {
    assert_eq!(SET_VIRTUAL_UART_PARAM, 164);
    let row = call::lookup(SET_VIRTUAL_UART_PARAM).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(
        row,
        Some(("set_virtual_uart_param", 3, 0, CallSource::Linux))
    );
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_164() {
    assert_eq!(VIRTUAL_UART_SET_PARAM_HANDLERS.len(), 1);
    let handler = virtual_uart_set_param_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x2F4100);
    assert_eq!(
        table_entry("4.93", SET_VIRTUAL_UART_PARAM),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_set_param_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(
            virtual_uart_set_param_handler(firmware),
            None,
            "{firmware:?}"
        );
    }
}

/// lv1 builds the three refusals as `(1 << 32) | 0xFFFFFFFA`,
/// `(2 << 32) | 0xFFFFFFFA` and `(4 << 32) | 0xFFFFFFEF`
/// [OFW-4.93 lv1 a:0x2F4038-0x2F4044] [OFW-4.93 lv1 a:0x2F3DF8-0x2F3E04]
/// [OFW-4.93 lv1 a:0x2F40EC-0x2F40F8].
#[test]
fn the_set_param_refusals_mark_r3_r4_and_r5() {
    assert_eq!(
        SET_VIRTUAL_UART_PARAM_NO_PORT_R3,
        (1 << 32) | zero_extended(LV1_NO_ENTRY)
    );
    assert_eq!(
        SET_VIRTUAL_UART_PARAM_NO_PARAM_R3,
        (2 << 32) | zero_extended(LV1_NO_ENTRY)
    );
    assert_eq!(
        SET_VIRTUAL_UART_PARAM_MASK_R3,
        (4 << 32) | zero_extended(LV1_ILLEGAL_PARAMETER_VALUE)
    );
}

/// The ids are the first three of Linux `enum vuart_param`. The worker
/// refuses an r4 above 2 [OFW-4.93 lv1 a:0x2F3CFC-0x2F3D08], and a mask
/// with a bit above bit 2 [OFW-4.93 lv1 a:0x2F3F40-0x2F3F48].
#[test]
fn the_settable_ids_are_0_to_2_and_the_mask_bound_is_7() {
    assert_eq!(VIRTUAL_UART_PARAM_TX_TRIGGER, 0);
    assert_eq!(VIRTUAL_UART_PARAM_RX_TRIGGER, 1);
    assert_eq!(VIRTUAL_UART_PARAM_INTERRUPT_MASK, 2);
    assert_eq!(VIRTUAL_UART_SET_PARAM_MAX, 2);
    assert_eq!(VIRTUAL_UART_INTERRUPT_MASK_MAX, 7);
}
