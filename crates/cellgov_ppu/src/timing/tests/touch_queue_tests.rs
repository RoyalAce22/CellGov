//! A load of a line that a touch still brings waits for the reloads that
//! later touches queue in its L1 set, until the set's reloads wait for
//! [`k::PPU_L1_SET_TOUCH_LIMIT`] touches.
//!
//! The evidence is the `ppu_issue_touch_queue` capture, which the doc of
//! [`k::PPU_L1_SET_TOUCH_LIMIT`] describes. No document or capture shows
//! whether a touch that hits the L1 ends the wait; the last test pins
//! the choice that `L1::behind` describes.

use super::{Caches, Level, Operand};
use cellgov_ps3_abi::hw::ppu_issue as k;

/// A line no test loads before it starts.
const LINE_A: u64 = 0x100_0000;
/// The bytes between two lines in one L1 set.
const SET_STRIDE: u64 = k::PPU_L1D_SETS as u64 * k::PPU_CACHE_LINE_BYTES;
/// The cycle after which every reload of a load at cycle 0 has written.
const SETTLED: u64 = 1000;

fn at(ea: u64) -> Operand {
    Operand::untranslated(ea, 8)
}

/// Line `i` of `LINE_A`'s L1 set, `LINE_A` first.
const fn in_set(i: u64) -> u64 {
    LINE_A + i * SET_STRIDE
}

/// Caches after a touch of `LINE_A` at cycle 10 and touches of the next
/// `others` lines of its set, one a cycle.
fn touched_then(others: u64) -> Caches {
    let mut caches = Caches::new();
    caches.touch(10, LINE_A, Some(LINE_A));
    for i in 1..=others {
        caches.touch(10 + i, in_set(i), Some(in_set(i)));
    }
    caches
}

#[test]
fn a_load_of_a_touched_line_starts_when_a_later_touch_reload_of_its_set_writes() {
    let mut caches = touched_then(1);
    let later = caches.arrival(12, at(in_set(1)));
    assert!(later > caches.arrival(12, at(LINE_A)));
    assert_eq!(
        caches.load_queued(12, at(LINE_A), false),
        (Level::L1, later)
    );
}

#[test]
fn a_load_of_a_touched_line_waits_for_its_touch_alone_once_its_set_holds_the_touch_limit() {
    let below = k::PPU_L1_SET_TOUCH_LIMIT as u64 - 2;
    let mut held_below = touched_then(below);
    let last = held_below.arrival(20, at(in_set(below)));
    assert_eq!(
        held_below.load_queued(20, at(LINE_A), false),
        (Level::L1, last)
    );
    let mut at_limit = touched_then(below + 1);
    assert_eq!(at_limit.load_queued(20, at(LINE_A), false), (Level::L1, 0));
}

#[test]
fn a_second_touch_of_a_line_on_its_way_counts_toward_the_touch_limit() {
    let below = k::PPU_L1_SET_TOUCH_LIMIT as u64 - 2;
    let mut caches = touched_then(below);
    caches.touch(19, LINE_A, Some(LINE_A));
    assert_eq!(caches.load_queued(20, at(LINE_A), false), (Level::L1, 0));
}

#[test]
fn a_touch_that_hits_the_l1_while_a_reload_of_its_set_is_on_its_way_ends_the_wait() {
    let held = in_set(2);
    let mut caches = Caches::new();
    caches.load_queued(0, at(held), false);
    assert_eq!(caches.holds(held, held), (true, true));
    caches.touch(SETTLED, LINE_A, Some(LINE_A));
    caches.touch(SETTLED + 1, in_set(1), Some(in_set(1)));
    caches.touch(SETTLED + 2, held, Some(held));
    assert_eq!(
        caches.load_queued(SETTLED + 3, at(LINE_A), false),
        (Level::L1, 0)
    );
}
