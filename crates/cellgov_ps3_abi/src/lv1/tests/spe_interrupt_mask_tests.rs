use super::*;
use crate::lv1::call::{self, CallSource, SET_SPE_INTERRUPT_MASK};
use crate::lv1::status::LV1_ILLEGAL_PARAMETER_VALUE;
use crate::lv1::table::{table_entry, TableEntry};

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 61.
#[test]
fn set_spe_interrupt_mask_carries_its_header_number_name_and_counts() {
    assert_eq!(SET_SPE_INTERRUPT_MASK, 61);
    let row = call::lookup(SET_SPE_INTERRUPT_MASK).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(
        row,
        Some(("set_spe_interrupt_mask", 3, 0, CallSource::Linux))
    );
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_61() {
    let handler = spe_interrupt_mask_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x2FE570);
    assert_eq!(
        table_entry("4.93", SET_SPE_INTERRUPT_MASK),
        Some(TableEntry::Handler(handler.handler))
    );
    assert_eq!(spe_interrupt_mask_handler("4.92"), None);
}

/// The worker builds the status as 2 << 32, then `oris` 0xFFFF and `ori`
/// 0xFFEF [OFW-4.93 lv1 a:0x2FE500-0x2FE50C].
#[test]
fn the_class_refusal_is_minus_17_with_bit_1_of_the_high_word_set() {
    assert_eq!(
        SET_SPE_INTERRUPT_MASK_CLASS_R3,
        (1 << 33) | zero_extended(LV1_ILLEGAL_PARAMETER_VALUE)
    );
}

/// The table's class 0 doubleword is 0xFFFFFFFE_FFFFFFF7, and the class 1
/// and 2 doublewords have each bit set
/// [OFW-4.93 lv1 a:0x329A98-0x329AAC].
#[test]
fn lv1_clears_two_bits_of_a_class_0_mask_and_no_bit_of_class_1_or_2() {
    assert_eq!(!SPE_INTERRUPT_CLASS_MASKS[0], 0x0000_0001_0000_0008);
    assert_eq!(SPE_INTERRUPT_CLASS_MASKS[1..], [u64::MAX; 2]);
}

/// The 4.93 kernel's boot masks are 7, 3 and 7 for classes 0 to 2
/// [OFW-4.93 lv2_kernel a:0x800000000004f81c]
/// [OFW-4.93 lv2_kernel a:0x800000000004f73c]
/// [OFW-4.93 lv2_kernel a:0x800000000004f7e4].
#[test]
fn lv1_keeps_each_bit_of_the_kernels_boot_masks() {
    let boot_masks = [7, 3, 7];
    for (class, mask) in boot_masks.into_iter().enumerate() {
        assert_eq!(
            mask & SPE_INTERRUPT_CLASS_MASKS[class],
            mask,
            "class {class}"
        );
    }
}
