//! The 4.93 port ids of the record calls held to the console capture of
//! the kernel's per-thread records. The README of the capture gives the
//! record's other fields; the port id's place is cited on [`PORT_ID`].

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

/// The record's thread index.
const INDEX: u64 = 0x00;
/// r4 out of hypercall 18, which the kernel stores
/// [OFW-4.93 lv2_kernel a:0x8000000000007de8].
const PORT_ID: u64 = 0x40;

fn record(index: u64, offset: u64) -> u64 {
    captured_doubleword_in(
        LOGICAL_PPE_RECORD_CAPTURE,
        READS,
        RECORDS + RECORD_SIZE * index + offset,
    )
}

#[test]
fn the_4_93_record_call_of_thread_0_gets_the_id_the_record_of_thread_0_kept() {
    assert_eq!(record(0, INDEX), 0);
    assert_eq!(
        game_os_port_id("4.93", 0, RECORD_SC),
        Some(record(0, PORT_ID))
    );
}

#[test]
fn the_4_93_record_call_of_thread_1_gets_the_id_the_record_of_thread_1_kept() {
    assert_eq!(record(1, INDEX), 1);
    assert_eq!(
        game_os_port_id("4.93", 1, RECORD_SC),
        Some(record(1, PORT_ID))
    );
}
