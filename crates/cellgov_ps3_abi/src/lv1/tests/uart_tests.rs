use super::*;
use crate::lv1::call::{self, CallSource, CONFIGURE_VIRTUAL_UART_IRQ};
use crate::lv1::status::{
    zero_extended, LV1_ILLEGAL_PARAMETER_VALUE, LV1_NO_ENTRY, LV1_TYPE_MISMATCH, LV1_WRONG_STATE,
};
use crate::lv1::table::{table_entry, TableEntry};

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 166.
#[test]
fn configure_virtual_uart_irq_carries_its_header_number_name_and_counts() {
    assert_eq!(CONFIGURE_VIRTUAL_UART_IRQ, 166);
    let row =
        call::lookup(CONFIGURE_VIRTUAL_UART_IRQ).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(
        row,
        Some(("configure_virtual_uart_irq", 1, 1, CallSource::Linux))
    );
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_166() {
    assert_eq!(VIRTUAL_UART_IRQ_HANDLERS.len(), 1);
    let handler = virtual_uart_irq_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x2F2810);
    assert_eq!(
        table_entry("4.93", CONFIGURE_VIRTUAL_UART_IRQ),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(virtual_uart_irq_handler(firmware), None, "{firmware:?}");
    }
}

#[test]
fn each_refusal_marks_r3_or_is_zero_extended() {
    let marked_r3 = |status: i64| (1u64 << 32) | zero_extended(status);
    assert_eq!(
        CONFIGURE_VIRTUAL_UART_IRQ_NO_REGION_R3,
        marked_r3(LV1_NO_ENTRY)
    );
    assert_eq!(
        CONFIGURE_VIRTUAL_UART_IRQ_REGION_R3,
        marked_r3(LV1_TYPE_MISMATCH)
    );
    assert_eq!(
        CONFIGURE_VIRTUAL_UART_IRQ_PAGE_R3,
        marked_r3(LV1_ILLEGAL_PARAMETER_VALUE)
    );
    assert_eq!(
        CONFIGURE_VIRTUAL_UART_IRQ_CONFIGURED_R3,
        zero_extended(LV1_WRONG_STATE)
    );
}

#[test]
fn the_address_bound_is_two_to_the_62_less_one() {
    assert_eq!(VIRTUAL_UART_IRQ_ADDRESS_MAX, (1 << 62) - 1);
}

/// lv1 compares the page of the first byte with the page of the byte at
/// +32 [OFW-4.93 lv1 a:0x2F292C-0x2F2940], so a bitmap that ends at a page
/// end fails.
#[test]
fn a_bitmap_passes_the_page_test_only_when_its_next_byte_shares_its_page() {
    let cases: &[(u64, bool)] = &[
        (0x0037_dc68, true),
        (0x0037_d000, true),
        (0x0037_dfdf, true),
        (0x0037_dfe0, false),
        (0x0037_dff8, false),
        (0x0037_e000, true),
        (u64::MAX - 8, false),
    ];
    assert_eq!(cases.len(), 7);
    for &(address, passes) in cases {
        assert_eq!(
            virtual_uart_irq_bitmap_in_small_page(address),
            passes,
            "{address:#x}"
        );
    }
}
