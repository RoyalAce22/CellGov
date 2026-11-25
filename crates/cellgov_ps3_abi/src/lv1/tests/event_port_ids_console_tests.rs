//! The 4.93 port id of each per-thread kernel call held to the console
//! capture of the eight port ids the kernel keeps for its threads. The
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

/// The captured port id of the call at `sc` on the hardware thread
/// `thread`: the record's +0x40, call A's +8, or call B's or call C's +0.
fn captured_id(thread: u8, sc: u64) -> u64 {
    let n = u64::from(thread);
    match sc {
        RECORD_SC => read(RECORDS + RECORD_SIZE * n + 0x40),
        START_A_SC => entry(TABLE_A, n, 8),
        START_B_SC => entry(TABLE_B, n, 0),
        START_C_SC => entry(TABLE_C, n, 0),
        _ => panic!("no captured slot for the call at {sc:#018x}"),
    }
}

#[test]
fn each_4_93_per_thread_call_gets_the_port_id_the_capture_reads_for_it() {
    for thread in 0..2 {
        assert_eq!(read(RECORDS + RECORD_SIZE * thread), thread, "record");
        assert_eq!(entry(TABLE_B, thread, 32), thread, "table B +32");
        assert_eq!(entry(TABLE_C, thread, 32), thread, "table C +32");
    }
    let all = game_os_port_calls("4.93").expect("the 4.93 port calls");
    assert!(
        all.windows(2).all(|pair| pair[0].id < pair[1].id),
        "in the order of the port id"
    );
    let per_thread = [RECORD_SC, START_A_SC, START_B_SC, START_C_SC];
    let calls: Vec<&GameOsPortCall> = all
        .iter()
        .filter(|call| per_thread.contains(&call.sc))
        .collect();
    assert_eq!(calls.len(), 8);
    for call in &calls {
        assert_eq!(
            captured_id(call.thread, call.sc),
            call.id,
            "thread {} at {:#018x}",
            call.thread,
            call.sc
        );
    }
    let mut sites: Vec<(u8, u64)> = all.iter().map(|c| (c.thread, c.sc)).collect();
    sites.sort_unstable();
    sites.dedup();
    assert_eq!(sites.len(), all.len(), "one row for each thread and site");
}

#[test]
fn the_4_93_rows_past_the_per_thread_calls_are_the_spe_record_network_and_storage_calls() {
    let all = game_os_port_calls("4.93").expect("the 4.93 port calls");
    let per_thread = [RECORD_SC, START_A_SC, START_B_SC, START_C_SC];
    let rest: Vec<(u8, u64)> = all
        .iter()
        .filter(|call| !per_thread.contains(&call.sc))
        .map(|call| (call.thread, call.sc))
        .collect();
    assert_eq!(
        rest,
        [
            (1, RECORD_START_SC),
            (0, SPE_SC),
            (0, RECORD_START_SC),
            (0, NETWORK_SC),
            (0, STORAGE_SC)
        ]
    );
}
