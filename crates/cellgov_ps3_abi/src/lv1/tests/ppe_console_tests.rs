//! The 4.93 logical PPE id held to the console capture of the kernel's
//! per-thread records. The README of the capture gives the offsets.

use super::*;
use crate::lv1::boot_record_capture::captured_doubleword_in;
use crate::lv1::interrupt::THREADS_PER_LOGICAL_PPE;

/// The reads of the capture's frame: three locator words, the record
/// count and 13 words of each of the two records.
const READS: usize = 30;

/// The address of the 4.93 kernel's first per-thread record.
const RECORDS: u64 = 0x8000_0000_0036_8f80;

/// The size of one record.
const RECORD_SIZE: u64 = 640;

/// The address of the 4.93 kernel's count of records it set up.
const COUNT: u64 = 0x8000_0000_0036_9488;

/// The record's thread index.
const INDEX: u64 = 0x00;
/// r4 out of hypercall 69: the logical PPE id.
const LOGICAL_PPE_ID: u64 = 0x08;

fn captured(address: u64) -> u64 {
    captured_doubleword_in(LOGICAL_PPE_RECORD_CAPTURE, READS, address)
}

fn record(index: u64, offset: u64) -> u64 {
    captured(RECORDS + RECORD_SIZE * index + offset)
}

#[test]
fn the_kernel_set_up_one_record_for_each_hardware_thread_of_the_logical_ppe() {
    assert_eq!(captured(COUNT), THREADS_PER_LOGICAL_PPE);
    for index in 0..THREADS_PER_LOGICAL_PPE {
        assert_eq!(record(index, INDEX), index, "record {index}");
    }
}

#[test]
fn the_4_93_logical_ppe_id_is_the_one_each_hardware_thread_kept() {
    for index in 0..THREADS_PER_LOGICAL_PPE {
        assert_eq!(
            game_os_logical_ppe_id("4.93"),
            Some(record(index, LOGICAL_PPE_ID)),
            "record {index}"
        );
    }
}
