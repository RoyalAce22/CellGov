use super::*;
use crate::lv1::call::{self, CallSource, READ_VIRTUAL_UART};
use crate::lv1::status::{zero_extended, LV1_NO_ENTRY, LV1_TYPE_MISMATCH};
use crate::lv1::table::{table_entry, TableEntry};

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 162.
#[test]
fn read_virtual_uart_carries_its_header_number_name_and_counts() {
    assert_eq!(READ_VIRTUAL_UART, 162);
    let row = call::lookup(READ_VIRTUAL_UART).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(row, Some(("read_virtual_uart", 3, 1, CallSource::Linux)));
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_162() {
    assert_eq!(VIRTUAL_UART_READ_HANDLERS.len(), 1);
    let handler = virtual_uart_read_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x2F429C);
    assert_eq!(
        table_entry("4.93", READ_VIRTUAL_UART),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_read_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(virtual_uart_read_handler(firmware), None, "{firmware:?}");
    }
}

/// lv1 builds the port refusal as `(1 << 32) | 0xFFFFFFFA`, the region
/// refusal as `(2 << 32) | 0xFFFFFFFA`, and the region check refusal as
/// `(2 << 32) | 0xFFFFFFF8` [OFW-4.93 lv1 a:0x2F4604-0x2F4610]
/// [OFW-4.93 lv1 a:0x2F4618-0x2F4624] [OFW-4.93 lv1 a:0x2F462C-0x2F4638].
#[test]
fn the_port_refusal_marks_r3_and_the_region_refusals_mark_r4() {
    assert_eq!(
        READ_VIRTUAL_UART_NO_PORT_R3,
        (1 << 32) | zero_extended(LV1_NO_ENTRY)
    );
    assert_eq!(
        READ_VIRTUAL_UART_NO_REGION_R3,
        (2 << 32) | zero_extended(LV1_NO_ENTRY)
    );
    assert_eq!(
        READ_VIRTUAL_UART_REGION_R3,
        (2 << 32) | zero_extended(LV1_TYPE_MISMATCH)
    );
}

/// The handler builds the bound as `clrldi r0, -1, 2`
/// [OFW-4.93 lv1 a:0x2F4344-0x2F4348].
#[test]
fn the_buffer_address_bound_is_2_to_the_62_less_1() {
    assert_eq!(VIRTUAL_UART_READ_ADDRESS_MAX, (1u64 << 62) - 1);
}
