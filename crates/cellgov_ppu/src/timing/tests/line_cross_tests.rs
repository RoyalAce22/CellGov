//! What the caches do for an access whose bytes reach the next line.

use super::cache::{Caches, Level};
use cellgov_ps3_abi::hw::ppu_issue as k;

/// The first of two adjacent lines; `LINE_B` is the second.
const LINE_A: u64 = 0x100_0000;
const LINE_B: u64 = LINE_A + k::PPU_CACHE_LINE_BYTES;

/// A doubleword whose last four bytes are in `LINE_B`.
const ACROSS: u64 = LINE_B - 4;

#[test]
fn a_load_into_a_missing_second_line_comes_from_memory_and_fills_it() {
    let mut caches = Caches::new();
    assert_eq!(caches.load(LINE_A, 8, false), Level::Memory);
    assert_eq!(caches.load(ACROSS, 8, false), Level::Memory);
    assert_eq!(caches.load(LINE_B, 8, false), Level::L1);
    assert_eq!(caches.load(ACROSS, 8, false), Level::L1);
}

#[test]
fn a_load_into_a_flushed_second_line_waits_for_the_flush() {
    let mut caches = Caches::new();
    caches.flush(0, LINE_B);
    assert_eq!(caches.busy(1, ACROSS, 8), k::PPU_DCBF_LINE_BUSY_CYCLES);
}

#[test]
fn a_load_into_a_touched_second_line_waits_for_the_touch() {
    let mut caches = Caches::new();
    caches.touch(0, LINE_B);
    assert_eq!(caches.arrival(1, ACROSS, 8), k::PPU_MEMORY_LOAD_USE_CYCLES);
}

#[test]
fn a_store_into_the_second_line_allocates_it_in_the_l2() {
    let mut caches = Caches::new();
    caches.store(ACROSS, 8);
    // With `skip_l1`, the load looks up the L2 alone.
    assert_eq!(caches.load(LINE_B, 8, true), Level::L2);
}
