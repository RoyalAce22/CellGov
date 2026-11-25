//! The 4.93 boot address space facts held to the console capture of the
//! kernel's record. The README of the capture gives the offsets.

use super::*;
use crate::lv1::boot_record_capture::captured_doubleword_in;

/// The reads of the capture's frame: two locator words and the record's
/// first 12 words.
const READS: usize = 14;

/// The address of the 4.93 kernel's virtual address space record.
const RECORD: u64 = 0x8000_0000_003e_3a80;

/// The vas id, r4 out of hypercall 2.
const VAS_ID: u64 = 0x30;
/// A constant the kernel stores [OFW-4.93 lv2_kernel a:0x800000000005dccc],
/// 0x800000000f000000.
const MARK: u64 = 0x40;
/// The page table size, r5 out of hypercall 2.
const HTAB_SIZE: u64 = 0x48;
/// The page table size shifted right by 7.
const HTAB_GROUPS: u64 = 0x50;

/// The r3 that the 4.93 kernel passes to hypercall 2
/// [OFW-4.93 lv2_kernel a:0x800000000005dce8].
const KERNEL_HTAB_SIZE_EXP: u64 = 0;

fn record(offset: u64) -> u64 {
    captured_doubleword_in(VAS_BOOT_RECORD_CAPTURE, READS, RECORD + offset)
}

#[test]
fn the_capture_reads_the_record_the_kernel_function_wrote() {
    assert_eq!(record(MARK), 0x8000_0000_0f00_0000);
    assert_eq!(record(HTAB_GROUPS), record(HTAB_SIZE) >> 7);
}

#[test]
fn the_4_93_boot_vas_id_is_the_one_the_kernel_kept() {
    assert_eq!(game_os_boot_vas_id("4.93"), Some(record(VAS_ID)));
}

#[test]
fn the_page_table_size_of_the_kernel_call_is_the_one_the_kernel_kept() {
    let exp = htab_size_exp(KERNEL_HTAB_SIZE_EXP).expect("the kernel's r3 passes the check");
    assert_eq!(htab_size(exp), record(HTAB_SIZE));
}
