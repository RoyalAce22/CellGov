use super::*;
use crate::lv1::call::{self, CallSource, CONNECT_IRQ_PLUG_EXT};
use crate::lv1::status::{zero_extended, LV1_NO_ENTRY, LV1_WRONG_STATE};
use crate::lv1::table::{table_entry, TableEntry};

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 12.
#[test]
fn connect_irq_plug_ext_carries_its_header_number_name_and_counts() {
    assert_eq!(CONNECT_IRQ_PLUG_EXT, 12);
    let row = call::lookup(CONNECT_IRQ_PLUG_EXT).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(row, Some(("connect_irq_plug_ext", 5, 0, CallSource::Linux)));
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_12() {
    assert_eq!(IRQ_PLUG_HANDLERS.len(), 1);
    let handler = irq_plug_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x2E9DA8);
    assert_eq!(
        table_entry("4.93", CONNECT_IRQ_PLUG_EXT),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(irq_plug_handler(firmware), None, "{firmware:?}");
    }
}

#[test]
fn plug_0_is_the_top_bit_of_the_first_status_doubleword() {
    assert_eq!(plug_status_bit(0), (0, 0x8000_0000_0000_0000));
    assert_eq!(plug_status_bit(1), (0, 0x4000_0000_0000_0000));
    assert_eq!(plug_status_bit(63), (0, 1));
    assert_eq!(plug_status_bit(64), (1, 0x8000_0000_0000_0000));
    assert_eq!(plug_status_bit(255), (3, 1));
}

#[test]
fn every_plug_has_its_own_bit_in_the_four_status_doublewords() {
    let mut seen = [0u64; 4];
    let mut count = 0;
    for plug in 0..=u8::MAX {
        let (word, mask) = plug_status_bit(plug);
        assert_eq!(mask.count_ones(), 1, "plug {plug}");
        assert_eq!(seen[word as usize] & mask, 0, "plug {plug}");
        seen[word as usize] |= mask;
        count += 1;
    }
    assert_eq!(count, IRQ_PLUGS_PER_THREAD);
    assert_eq!(seen, [u64::MAX; 4]);
}

#[test]
fn each_refusal_marks_the_argument_it_names() {
    let marked = |register: u32, status: i64| (1u64 << (32 + register - 3)) | zero_extended(status);
    assert_eq!(IRQ_PLUG_PPE_ID_R3, marked(3, LV1_NO_ENTRY));
    assert_eq!(IRQ_PLUG_THREAD_R3, marked(4, LV1_NO_ENTRY));
    assert_eq!(IRQ_PLUG_PLUG_R3, marked(5, LV1_NO_ENTRY));
    assert_eq!(IRQ_PLUG_PORT_R3, marked(6, LV1_NO_ENTRY));
    assert_eq!(IRQ_PLUG_R7_R3, marked(7, LV1_NO_ENTRY));
    assert_eq!(IRQ_PLUG_PORT_CONNECTED_R3, marked(6, LV1_WRONG_STATE));
    assert_eq!(IRQ_PLUG_IN_USE_R3, marked(5, LV1_WRONG_STATE));
}
