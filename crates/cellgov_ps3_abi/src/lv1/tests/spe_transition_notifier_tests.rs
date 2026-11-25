use super::*;
use crate::lv1::call::{self, CallSource, SET_SPE_TRANSITION_NOTIFIER};
use crate::lv1::status::LV1_ILLEGAL_PARAMETER_VALUE;
use crate::lv1::table::{table_entry, TableEntry};

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 64.
#[test]
fn set_spe_transition_notifier_carries_its_header_number_name_and_counts() {
    assert_eq!(SET_SPE_TRANSITION_NOTIFIER, 64);
    let row =
        call::lookup(SET_SPE_TRANSITION_NOTIFIER).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(
        row,
        Some(("set_spe_transition_notifier", 3, 0, CallSource::Linux))
    );
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_64() {
    let handler = spe_transition_notifier_handler("4.93").expect("the 4.93 handler");
    assert_eq!(
        table_entry("4.93", SET_SPE_TRANSITION_NOTIFIER),
        Some(TableEntry::Handler(handler.handler))
    );
    assert_eq!(spe_transition_notifier_handler("4.92"), None);
}

/// The method builds the status as 2 << 32, then `oris` 0xFFFF and `ori`
/// 0xFFEF [OFW-4.93 lv1 a:0x2FA4FC-0x2FA528].
#[test]
fn the_mask_refusal_is_minus_17_with_bit_1_of_the_high_word_set() {
    assert_eq!(
        SET_SPE_TRANSITION_NOTIFIER_MASK_R3,
        (1 << 33) | zero_extended(LV1_ILLEGAL_PARAMETER_VALUE)
    );
}

/// The 4.93 kernel's boot call passes the mask 1
/// [OFW-4.93 lv2_kernel a:0x800000000004f22c].
#[test]
fn the_type_1_mask_accepts_the_kernels_boot_mask_and_no_bit_above_2() {
    assert_eq!(LOGICAL_SPE_TYPE_1_TRANSITION_MASK, 7);
    assert_eq!(1 & !LOGICAL_SPE_TYPE_1_TRANSITION_MASK, 0);
    assert_ne!(0b1000 & !LOGICAL_SPE_TYPE_1_TRANSITION_MASK, 0);
}
