//! A touch's or an L1-missing load's reload holds its L1 set, and a later
//! miss to the set waits.
//!
//! [CBE-Handbook p:137 s:6.1.3.7] a load-miss queue entry takes a new request 7 cycles after the final data beat of its reload comes back from the L2.

use super::{Caches, Level, Operand};
use cellgov_ps3_abi::hw::ppu_issue as k;

/// A line no test loads before it starts.
const LINE_A: u64 = 0x100_0000;
/// The bytes between two lines in one L1 set.
const SET_STRIDE: u64 = k::PPU_L1D_SETS as u64 * k::PPU_CACHE_LINE_BYTES;
/// Another line in the L1 set of `LINE_A`.
const LINE_B: u64 = LINE_A + SET_STRIDE;
/// A line in the next L1 set.
const LINE_C: u64 = LINE_A + k::PPU_CACHE_LINE_BYTES;

/// The cycle the L1 set frees after a reload from memory sent at `sent`.
const fn freed_after_memory(sent: u64) -> u64 {
    sent + k::PPU_MEMORY_LOAD_USE_CYCLES + k::PPU_LOAD_MISS_ENTRY_REUSE_CYCLES
}

fn at(ea: u64) -> Operand {
    Operand::untranslated(ea, 8)
}

/// An effective address of `LINE_A`'s real line in the other congruence
/// class of its L1 set.
const LINE_A_OTHER_CLASS: u64 = LINE_A + SET_STRIDE / 2;

/// Caches after a touch of `LINE_A` at cycle 10 and a touch at cycle 11
/// of its real line through `LINE_A_OTHER_CLASS`, which takes the line
/// out of `LINE_A`'s L1 set.
///
/// [CBE-Handbook p:138 s:6.1.3.10] A line found at the other congruence class is invalidated, and the data from the L2 is placed at the congruence class of the effective address.
fn line_a_pushed_out_while_on_its_way() -> Caches {
    let mut caches = Caches::new();
    caches.touch(10, LINE_A, Some(LINE_A));
    caches.touch(11, LINE_A_OTHER_CLASS, Some(LINE_A));
    assert_eq!(caches.holds(LINE_A, LINE_A), (false, true));
    caches
}

#[test]
fn a_touch_in_a_set_that_a_reload_holds_sends_its_request_when_the_set_frees() {
    let mut caches = Caches::new();
    caches.touch(10, LINE_A, Some(LINE_A));
    assert_eq!(caches.reload_free(LINE_A), freed_after_memory(10));
    caches.touch(11, LINE_B, Some(LINE_B));
    assert_eq!(
        caches.arrival(12, at(LINE_B)),
        freed_after_memory(10) + k::PPU_MEMORY_LOAD_USE_CYCLES
    );
    assert_eq!(
        caches.reload_free(LINE_B),
        freed_after_memory(freed_after_memory(10))
    );
}

#[test]
fn a_touch_in_another_set_waits_only_for_the_touch_spacing() {
    let mut caches = Caches::new();
    caches.touch(10, LINE_A, Some(LINE_A));
    caches.touch(11, LINE_C, Some(LINE_C));
    let sent = 10 + k::PPU_TOUCH_INTERVAL_CYCLES;
    assert_eq!(
        caches.arrival(12, at(LINE_C)),
        sent + k::PPU_MEMORY_LOAD_USE_CYCLES
    );
}

#[test]
fn a_load_that_misses_a_set_a_reload_holds_starts_when_the_set_frees() {
    let mut caches = Caches::new();
    caches.touch(10, LINE_A, Some(LINE_A));
    assert_eq!(
        caches.load_queued(11, at(LINE_B), false),
        (Level::Memory, freed_after_memory(10))
    );
}

#[test]
fn a_load_that_misses_a_line_still_on_its_way_waits_for_no_set() {
    let mut caches = line_a_pushed_out_while_on_its_way();
    assert!(caches.reload_free(LINE_A) > 12);
    assert_eq!(caches.load_queued(12, at(LINE_A), false), (Level::L2, 0));
}

#[test]
fn a_load_of_a_touched_line_starts_no_earlier_than_the_request_its_set_held() {
    let mut caches = Caches::new();
    caches.touch(10, LINE_B, Some(LINE_B));
    caches.touch(11, LINE_A, Some(LINE_A));
    assert_eq!(
        caches.load_queued(12, at(LINE_A), false),
        (Level::L1, freed_after_memory(10))
    );
}

#[test]
fn a_touch_of_a_line_still_on_its_way_waits_for_no_set_and_holds_none() {
    let mut caches = line_a_pushed_out_while_on_its_way();
    let free = caches.reload_free(LINE_A);
    caches.touch(20, LINE_A, Some(LINE_A));
    assert_eq!(caches.reload_free(LINE_A), free);
    assert_eq!(
        caches.arrival(21, at(LINE_A)),
        10 + k::PPU_MEMORY_LOAD_USE_CYCLES
    );
}

#[test]
fn a_stall_delays_a_sets_next_free_reload_only_when_it_began_before_it() {
    let mut caches = Caches::new();
    caches.touch(10, LINE_A, Some(LINE_A));
    caches.stretch_touches(100, 50);
    assert_eq!(caches.reload_free(LINE_A), freed_after_memory(10) + 50);
    caches.stretch_touches(freed_after_memory(10) + 50, 50);
    assert_eq!(caches.reload_free(LINE_A), freed_after_memory(10) + 50);
}

#[test]
fn a_load_that_skips_the_l1_waits_for_its_set_and_then_holds_it() {
    let mut caches = Caches::new();
    caches.touch(10, LINE_A, Some(LINE_A));
    assert_eq!(
        caches.load_queued(11, at(LINE_B), true),
        (Level::Memory, freed_after_memory(10))
    );
    assert_eq!(
        caches.reload_free(LINE_B),
        freed_after_memory(freed_after_memory(10))
    );
}

#[test]
fn a_load_that_misses_the_l1_holds_its_set_for_its_trip_from_memory() {
    let mut caches = Caches::new();
    assert_eq!(
        caches.load_queued(10, at(LINE_A), false),
        (Level::Memory, 0)
    );
    assert_eq!(caches.reload_free(LINE_A), freed_after_memory(10));
    assert_eq!(caches.reload_free(LINE_C), 0);
}

#[test]
fn a_load_that_misses_the_l1_and_hits_the_l2_holds_its_set_for_its_trip_from_the_l2() {
    let mut caches = Caches::new();
    caches.load_queued(10, at(LINE_A), false);
    for i in 1..=k::PPU_L1D_WAYS as u64 {
        caches.load_queued(10, at(LINE_A + i * SET_STRIDE), false);
    }
    let free = caches.reload_free(LINE_A);
    assert_eq!(
        caches.load_queued(free, at(LINE_A), false),
        (Level::L2, free)
    );
    assert_eq!(
        caches.reload_free(LINE_A),
        free + k::PPU_L2_LOAD_USE_CYCLES + k::PPU_LOAD_MISS_ENTRY_REUSE_CYCLES
    );
}

#[test]
fn a_load_that_hits_the_l1_holds_no_set() {
    let mut caches = Caches::new();
    caches.load_queued(10, at(LINE_A), false);
    let free = caches.reload_free(LINE_A);
    assert_eq!(
        caches.load_queued(free + 1, at(LINE_A), false),
        (Level::L1, 0)
    );
    assert_eq!(caches.reload_free(LINE_A), free);
}

#[test]
fn a_load_that_misses_a_line_still_on_its_way_holds_no_set() {
    let mut caches = line_a_pushed_out_while_on_its_way();
    let free = caches.reload_free(LINE_A);
    caches.load_queued(12, at(LINE_A), false);
    assert_eq!(caches.reload_free(LINE_A), free);
}
