//! The virtual address space checks hold the Linux-named status in the
//! low word and the failing argument's mark in the high word, and their
//! bounds hold lv1's compare values.

use super::*;
use crate::lv1::status::{zero_extended, LV1_ILLEGAL_PARAMETER_VALUE};
use crate::lv1::table::{table_entry, TableEntry};

/// The page shifts the 4.93 kernel passes in r5
/// [OFW-4.93 lv2_kernel a:0x800000000005dcc0-0x800000000005dce4]: 16 and
/// 20.
const KERNEL_PAGE_SHIFTS: u64 = 0x1014_0000_0000_0000;

fn marked(mark: u64, status: i64) -> u64 {
    (mark << 32) | zero_extended(status)
}

#[test]
fn each_check_result_marks_its_argument_over_illegal_parameter_value() {
    let illegal = LV1_ILLEGAL_PARAMETER_VALUE;
    assert_eq!(CONSTRUCT_VAS_HTAB_SIZE_EXP_R3, marked(0b001, illegal));
    assert_eq!(CONSTRUCT_VAS_PAGE_SIZE_COUNT_R3, marked(0b010, illegal));
    assert_eq!(CONSTRUCT_VAS_PAGE_SHIFT_R3, marked(0b100, illegal));
}

#[test]
fn the_4_93_handlers_are_the_live_table_entries_of_2_and_7() {
    let handlers = address_space_handlers("4.93").expect("4.93 handlers");
    assert_eq!(
        table_entry("4.93", 2),
        Some(TableEntry::Handler(handlers.construct))
    );
    assert_eq!(
        table_entry("4.93", 7),
        Some(TableEntry::Handler(handlers.select))
    );
    assert_eq!(address_space_handlers("4.92"), None);
}

#[test]
fn an_exponent_below_eighteen_rises_to_eighteen_and_one_above_twenty_fails() {
    assert_eq!(htab_size_exp(0), Some(18));
    assert_eq!(htab_size_exp(17), Some(18));
    assert_eq!(htab_size_exp(18), Some(18));
    assert_eq!(htab_size_exp(19), Some(19));
    assert_eq!(htab_size_exp(20), Some(20));
    assert_eq!(htab_size_exp(21), None);
    assert_eq!(htab_size_exp(u64::MAX), None);
}

#[test]
fn the_page_table_size_is_two_to_the_exponent() {
    assert_eq!(htab_size(18), 0x4_0000);
    assert_eq!(htab_size(20), 0x10_0000);
}

#[test]
fn the_kernel_page_shifts_pass_and_a_shift_of_twelve_fails() {
    assert!(page_shifts_accepted(2, KERNEL_PAGE_SHIFTS));
    assert!(page_shifts_accepted(1, 0x1800_0000_0000_0000));
    assert!(!page_shifts_accepted(1, 0x0C00_0000_0000_0000));
    assert!(!page_shifts_accepted(2, 0x100C_0000_0000_0000));
}

#[test]
fn only_the_first_count_shifts_are_checked() {
    assert!(page_shifts_accepted(0, u64::MAX));
    assert!(page_shifts_accepted(1, 0x10FF_FFFF_FFFF_FFFF));
    assert!(!page_shifts_accepted(2, 0x10FF_FFFF_FFFF_FFFF));
}

#[test]
fn a_count_above_two_fails_the_page_shift_check_whatever_the_shifts() {
    assert!(!page_shifts_accepted(3, 0x1010_1000_0000_0000));
    assert!(!page_shifts_accepted(u64::MAX, KERNEL_PAGE_SHIFTS));
}
