use super::*;
use crate::lv1::call::{self, CallSource, CONSTRUCT_IO_IRQ_OUTLET};
use crate::lv1::table::{table_entry, TableEntry};

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 120.
#[test]
fn construct_io_irq_outlet_carries_its_header_number_name_and_counts() {
    assert_eq!(CONSTRUCT_IO_IRQ_OUTLET, 120);
    let row =
        call::lookup(CONSTRUCT_IO_IRQ_OUTLET).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(
        row,
        Some(("construct_io_irq_outlet", 1, 1, CallSource::Linux))
    );
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_120() {
    assert_eq!(IO_IRQ_OUTLET_HANDLERS.len(), 1);
    let handler = io_irq_outlet_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x2C0604);
    assert_eq!(
        table_entry("4.93", CONSTRUCT_IO_IRQ_OUTLET),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_io_irq_outlet_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(io_irq_outlet_handler(firmware), None, "{firmware:?}");
    }
}

/// [OFW-4.93 lv1 a:0x2C06B0] `cmpldi cr7, r27, 0x3f`.
#[test]
fn the_largest_accepted_interrupt_id_is_63() {
    assert_eq!(IO_IRQ_INTERRUPT_ID_MAX, 0x3F);
}

/// [OFW-4.93 lv1 a:0x2C092C-0x2C093C] `li 1; sldi 32; oris 0xffff; ori 0xfffa`.
#[test]
fn an_interrupt_id_above_63_returns_no_entry_with_r3_marked() {
    assert_eq!(IO_IRQ_OUTLET_ID_R3, 0x0000_0001_FFFF_FFFA);
}

/// [OFW-4.93 lv1 a:0x2C0940-0x2C094C] `li 1; sldi 32; oris 0xffff; ori 0xfff7`.
#[test]
fn a_failed_bind_returns_busy_with_r3_marked() {
    assert_eq!(IO_IRQ_OUTLET_BUSY_R3, 0x0000_0001_FFFF_FFF7);
}
