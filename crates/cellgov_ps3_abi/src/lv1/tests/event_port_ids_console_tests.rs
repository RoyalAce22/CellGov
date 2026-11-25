//! The 4.93 first port id and the object table's first-free-id rule held
//! to the console capture of all eight port ids the kernel keeps. The
//! README of the capture cites each place the kernel stores an id.

use super::*;
use crate::lv1::boot_record_capture::captured_doubleword_in;

const CAPTURE: &str = "tests/micro/lv2_event_port_ids/ps3/cech20-cex-493";

/// The reads of the capture's frame: five locator words, two words of
/// each record and the entries of the three tables.
const READS: usize = 45;

/// The kernel's per-thread records and the size of one record.
const RECORDS: u64 = 0x8000_0000_0036_8f80;
const RECORD_SIZE: u64 = 640;

/// The three per-thread port tables and the size of one entry in each.
const TABLE_A: (u64, u64) = (0x8000_0000_0037_a668, 48);
const TABLE_B: (u64, u64) = (0x8000_0000_003d_ded0, 56);
const TABLE_C: (u64, u64) = (0x8000_0000_0040_67d0, 40);

fn read(address: u64) -> u64 {
    captured_doubleword_in(CAPTURE, READS, address)
}

fn entry((table, size): (u64, u64), thread: u64, offset: u64) -> u64 {
    read(table + size * thread + offset)
}

/// The port id of each of the kernel's eight calls: per thread, the
/// record's +0x40, call A's +8, and call B's and call C's +0.
fn port_ids() -> Vec<u64> {
    (0..2)
        .flat_map(|thread| {
            [
                read(RECORDS + RECORD_SIZE * thread + 0x40),
                entry(TABLE_A, thread, 8),
                entry(TABLE_B, thread, 0),
                entry(TABLE_C, thread, 0),
            ]
        })
        .collect()
}

#[test]
fn the_kernel_s_eight_ports_take_the_ids_that_run_on_from_the_first_port_id() {
    for thread in 0..2 {
        assert_eq!(read(RECORDS + RECORD_SIZE * thread), thread, "record");
        assert_eq!(entry(TABLE_B, thread, 32), thread, "table B +32");
        assert_eq!(entry(TABLE_C, thread, 32), thread, "table C +32");
    }
    let first = game_os_first_port_id("4.93").expect("the 4.93 first port id");
    assert_eq!(read(RECORDS + 0x40), first, "the record of thread 0");
    let mut ids = port_ids();
    ids.sort_unstable();
    assert_eq!(ids, (first..first + 8).collect::<Vec<u64>>());
}
