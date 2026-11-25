use super::*;
use crate::lv1::call::{self, CallSource, CONFIGURE_IRQ_STATE_BITMAP};
use crate::lv1::memory::region_address;
use crate::lv1::status::{
    zero_extended, LV1_ILLEGAL_PARAMETER_VALUE, LV1_NO_ENTRY, LV1_TYPE_MISMATCH, LV1_WRONG_STATE,
};
use crate::lv1::table::{table_entry, TableEntry};

fn marked(mark: u64, status: i64) -> u64 {
    (mark << 32) | zero_extended(status)
}

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 11.
#[test]
fn configure_irq_state_bitmap_carries_its_header_number_name_and_counts() {
    assert_eq!(CONFIGURE_IRQ_STATE_BITMAP, 11);
    let row =
        call::lookup(CONFIGURE_IRQ_STATE_BITMAP).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(
        row,
        Some(("configure_irq_state_bitmap", 3, 0, CallSource::Linux))
    );
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_11() {
    assert_eq!(IRQ_STATE_BITMAP_HANDLERS.len(), 1);
    let handler = irq_state_bitmap_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x2EA35C);
    assert_eq!(
        table_entry("4.93", CONFIGURE_IRQ_STATE_BITMAP),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    assert_eq!(irq_state_bitmap_handler("3.70"), None);
    assert_eq!(irq_state_bitmap_handler("4.92"), None);
    assert_eq!(irq_state_bitmap_handler(""), None);
}

#[test]
fn each_check_result_marks_its_argument_over_the_status_lv1_builds() {
    assert_eq!(IRQ_STATE_BITMAP_PPE_ID_R3, marked(0b001, LV1_NO_ENTRY));
    assert_eq!(IRQ_STATE_BITMAP_THREAD_R3, marked(0b010, LV1_NO_ENTRY));
    assert_eq!(
        IRQ_STATE_BITMAP_NOT_REGISTERED_R3,
        marked(0, LV1_WRONG_STATE)
    );
    assert_eq!(
        IRQ_STATE_BITMAP_ALIGNMENT_R3,
        marked(0b100, LV1_ILLEGAL_PARAMETER_VALUE)
    );
    assert_eq!(IRQ_STATE_BITMAP_NO_REGION_R3, marked(0b100, LV1_NO_ENTRY));
    assert_eq!(IRQ_STATE_BITMAP_REGION_R3, marked(0b100, LV1_TYPE_MISMATCH));
}

/// Linux `struct ps3_bmp`: four status doublewords, then the mask
/// doubleword and three unused ones.
#[test]
fn a_bitmap_is_64_bytes_of_which_the_first_32_are_status() {
    assert_eq!(IRQ_STATE_BITMAP_BYTES, 64);
    assert_eq!(IRQ_STATUS_BYTES, 32);
}

#[test]
fn the_handler_takes_an_8_byte_aligned_bitmap_for_one_of_2_threads() {
    assert_eq!(IRQ_STATE_BITMAP_ALIGN, 8);
    assert_eq!(THREADS_PER_LOGICAL_PPE, 2);
}

#[test]
fn an_address_with_size_exponent_0_below_bit_42_looks_up_the_region_at_0() {
    assert_eq!(region_lookup_base(0), 0);
    assert_eq!(region_lookup_base(0x0036_9140), 0);
    assert_eq!(region_lookup_base((1 << 42) - 8), 0);
}

#[test]
fn an_address_with_size_exponent_0_keeps_its_bits_from_bit_48() {
    assert_eq!(
        region_lookup_base(0x0001_0000_0036_9140),
        0x0001_0000_0000_0000
    );
}

#[test]
fn the_top_address_has_size_exponent_63_and_keeps_only_bit_63() {
    assert_eq!(region_lookup_base(u64::MAX), 1 << 63);
}

#[test]
fn an_address_in_an_encoded_region_looks_up_the_region_address() {
    let region = region_address(0x10_0000, 2).expect("a 1 MiB region at counter 2");
    assert_eq!(region, 0x5000_0020_0000);
    assert_eq!(region_lookup_base(region), region);
    assert_eq!(region_lookup_base(region + 0xF_FFF8), region);
    assert_eq!(region_lookup_base(region + 0x10_0000), region + 0x10_0000);
}
