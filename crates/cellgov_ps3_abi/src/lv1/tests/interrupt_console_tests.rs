//! The thread check held to the console capture of the 4.93 kernel's
//! per-thread records. The kernel loads a record's +0x10 into r4 of
//! hypercall 11 [OFW-4.93 lv2_kernel a:0x8000000000007d70].

use super::*;
use crate::lv1::boot_record_capture::captured_doubleword_in;
use crate::lv1::ppe::LOGICAL_PPE_RECORD_CAPTURE;

/// The reads of the capture's frame: three locator words, the record
/// count and 13 words of each of the two records.
const READS: usize = 30;

/// The address of the 4.93 kernel's first per-thread record.
const RECORDS: u64 = 0x8000_0000_0036_8f80;

/// The size of one record.
const RECORD_SIZE: u64 = 640;

/// The address of the 4.93 kernel's count of the records that it builds.
const COUNT: u64 = 0x8000_0000_0036_9488;

/// The record's thread index.
const INDEX: u64 = 0x00;

/// r4 of hypercall 11: `cntlzw` of CTRL.
const THREAD: u64 = 0x10;

fn captured(address: u64) -> u64 {
    captured_doubleword_in(LOGICAL_PPE_RECORD_CAPTURE, READS, address)
}

fn record(index: u64, offset: u64) -> u64 {
    captured(RECORDS + RECORD_SIZE * index + offset)
}

#[test]
fn each_hardware_thread_registers_its_own_thread_below_the_thread_count() {
    let records = captured(COUNT);
    assert_eq!(records, THREADS_PER_LOGICAL_PPE, "one record per thread");
    for index in 0..records {
        assert_eq!(record(index, INDEX), index, "record {index}");
        let thread = record(index, THREAD);
        assert_eq!(thread, index, "record {index}");
        assert!(thread < THREADS_PER_LOGICAL_PPE, "record {index}");
    }
}
