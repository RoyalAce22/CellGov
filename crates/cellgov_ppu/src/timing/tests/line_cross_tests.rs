//! What the caches do for an access whose bytes reach the next line.

use super::cache::{Caches, Level, Operand};
use cellgov_ps3_abi::hw::ppu_issue as k;

/// The first of two adjacent lines; `LINE_B` is the second.
const LINE_A: u64 = 0x100_0000;
const LINE_B: u64 = LINE_A + k::PPU_CACHE_LINE_BYTES;

/// A doubleword whose last four bytes are in `LINE_B`.
const ACROSS: u64 = LINE_B - 4;

/// The distance from each real address below to its effective address on
/// a translating thread.
const EA_OFFSET: u64 = 0xC000_0000;

/// [`ACROSS`] on a translating thread: its real lines are `LINE_A` and
/// `LINE_B`, and its effective lines are other lines.
const TRANSLATED_ACROSS: Operand = Operand {
    ea: ACROSS + EA_OFFSET,
    ra: ACROSS,
    len: 8,
    tail: None,
};

#[test]
fn a_load_into_a_missing_second_line_comes_from_memory_and_fills_it() {
    let mut caches = Caches::new();
    assert_eq!(
        caches.load(Operand::untranslated(LINE_A, 8), false),
        Level::Memory
    );
    assert_eq!(
        caches.load(Operand::untranslated(ACROSS, 8), false),
        Level::Memory
    );
    assert_eq!(
        caches.load(Operand::untranslated(LINE_B, 8), false),
        Level::L1
    );
    assert_eq!(
        caches.load(Operand::untranslated(ACROSS, 8), false),
        Level::L1
    );
}

#[test]
fn a_load_into_a_flushed_second_line_waits_for_the_flush() {
    let mut caches = Caches::new();
    caches.flush(0, LINE_B);
    assert_eq!(
        caches.busy(1, Operand::untranslated(ACROSS, 8)),
        k::PPU_DCBF_LINE_BUSY_CYCLES
    );
}

#[test]
fn a_load_into_a_touched_second_line_waits_for_the_touch() {
    let mut caches = Caches::new();
    caches.touch(0, LINE_B, Some(LINE_B));
    assert_eq!(
        caches.arrival(1, Operand::untranslated(ACROSS, 8)),
        k::PPU_MEMORY_LOAD_USE_CYCLES
    );
}

#[test]
fn a_store_into_the_second_line_allocates_it_in_the_l2() {
    let mut caches = Caches::new();
    caches.store(Operand::untranslated(ACROSS, 8));
    // With `skip_l1`, the load looks up the L2 alone.
    assert_eq!(
        caches.load(Operand::untranslated(LINE_B, 8), true),
        Level::L2
    );
}

#[test]
fn a_load_of_the_last_doubleword_of_a_line_leaves_the_next_line_out() {
    let mut caches = Caches::new();
    assert_eq!(
        caches.load(Operand::untranslated(LINE_B - 8, 8), false),
        Level::Memory
    );
    assert_eq!(
        caches.load(Operand::untranslated(LINE_B, 8), false),
        Level::Memory
    );
}

#[test]
fn a_load_across_into_the_last_line_below_2_to_the_64_fills_that_line() {
    let mut caches = Caches::new();
    let last_line = u64::MAX - (k::PPU_CACHE_LINE_BYTES - 1);
    assert_eq!(
        caches.load(Operand::untranslated(last_line - 4, 8), false),
        Level::Memory
    );
    assert_eq!(
        caches.load(Operand::untranslated(last_line, 8), false),
        Level::L1
    );
}

/// [CBE-Handbook p:140 s:6.1.5.1] The L2 has a real-address index and real-address tags.
#[test]
fn a_translated_crossing_load_waits_for_a_flush_of_its_first_real_line() {
    let mut caches = Caches::new();
    caches.flush(0, LINE_A);
    assert_eq!(
        caches.busy(1, TRANSLATED_ACROSS),
        k::PPU_DCBF_LINE_BUSY_CYCLES
    );
}

/// [CBE-Handbook p:140 s:6.1.5.1] The L2 has a real-address index and real-address tags.
#[test]
fn a_translated_crossing_load_waits_for_a_touch_of_its_first_real_line() {
    let mut caches = Caches::new();
    caches.touch(0, LINE_A + EA_OFFSET, Some(LINE_A));
    assert_eq!(
        caches.arrival(1, TRANSLATED_ACROSS),
        k::PPU_MEMORY_LOAD_USE_CYCLES
    );
}
