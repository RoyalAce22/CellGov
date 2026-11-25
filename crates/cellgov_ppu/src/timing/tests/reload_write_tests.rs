//! A reload writes its line into the L1 when its data returns. Until
//! then, the line that it will evict stays in the L1.
//!
//! [CBE-Handbook p:137 s:6.1.3.7] The DCache tag array is updated after the reload is written to the DCache.

use super::{Caches, Level, Operand, LINE_CYCLES};
use cellgov_ps3_abi::hw::ppu_issue as k;

/// A line no test loads before it starts, in L1 set 0.
const LINE_A: u64 = 0x100_0000;
/// The bytes between two lines in one L1 set.
const SET_STRIDE: u64 = k::PPU_L1D_SETS as u64 * k::PPU_CACHE_LINE_BYTES;
/// Cycles that outlast a reload from memory and the hold it puts on its
/// L1 set.
const SETTLED: u64 = k::PPU_MEMORY_LOAD_USE_CYCLES + k::PPU_LOAD_MISS_ENTRY_REUSE_CYCLES + 1;

/// The lines of `LINE_A`'s L1 set that each pass touches.
const REVISITED: u64 = 8;
/// Cycles between the starts of two passes: the reloads of one pass
/// return before the next pass starts.
const PASS: u64 = (REVISITED + 1) * SETTLED;

fn at(ea: u64) -> Operand {
    Operand::untranslated(ea, 8)
}

/// Line `i` of `LINE_A`'s L1 set.
const fn in_set(i: u64) -> u64 {
    LINE_A + i * SET_STRIDE
}

/// Caches whose set of `LINE_A` holds its first 4 lines, loaded one at a
/// time, and a cycle after the last reload returns. The tree's next
/// victim is line 0.
fn full_set() -> (Caches, u64) {
    let mut caches = Caches::new();
    let mut t = 0;
    for i in 0..k::PPU_L1D_WAYS as u64 {
        caches.load_queued(t, at(in_set(i)), false);
        t += SETTLED;
    }
    (caches, t)
}

#[test]
fn a_line_stays_in_the_l1_until_the_reload_that_evicts_it_writes() {
    let (mut early, t) = full_set();
    early.touch(t, in_set(4), Some(in_set(4)));
    let mut late = early.clone();
    assert_eq!(early.load_queued(t + 1, at(in_set(0)), false).0, Level::L1);
    assert_eq!(
        late.load_queued(t + SETTLED, at(in_set(0)), false).0,
        Level::L2
    );
}

/// The requests that the `REVISITED` touches of the last of `passes`
/// passes send. Each pass ends in a load of a new line in the set.
fn requests_in_last_pass(passes: u64) -> u64 {
    let mut caches = Caches::new();
    let mut sent = 0;
    for p in 0..passes {
        let start = p * PASS;
        sent = 0;
        for i in 0..REVISITED {
            let before = caches.touch_free();
            caches.touch(start + i, in_set(i), Some(in_set(i)));
            sent += u64::from(caches.touch_free() != before);
        }
        caches.load_queued(start + REVISITED, at(in_set(REVISITED + 1 + p)), false);
    }
    sent
}

/// In the `ppu_issue_touch_reload` capture, loop 3 takes 643 cycles a
/// load, about what 5 touch reloads in the load's set cost.
#[test]
fn eight_touches_that_revisit_the_same_lines_of_one_l1_set_send_five_requests() {
    for passes in 2..=4 {
        assert_eq!(requests_in_last_pass(passes), 5, "after {passes} passes");
    }
}

/// Where a load of line 0 of a full set finds it, after a touch of a
/// fifth line in the set and `others` touches of lines in other sets.
fn victim_after(others: u64) -> Level {
    let (mut caches, t) = full_set();
    caches.touch(t, in_set(4), Some(in_set(4)));
    for j in 1..=others {
        let line = LINE_A + j * k::PPU_CACHE_LINE_BYTES;
        caches.touch(t + j, line, Some(line));
    }
    caches.load_queued(t + others + 1, at(in_set(0)), false).0
}

#[test]
fn a_reload_past_the_tables_size_writes_the_first_returning_line_at_once() {
    assert_eq!(victim_after(LINE_CYCLES as u64 - 1), Level::L1);
    assert_eq!(victim_after(LINE_CYCLES as u64), Level::L2);
}

/// [CBE-Handbook p:138 s:6.1.3.10] The copy at the other congruence class is invalidated, and then the data comes in from the L2.
#[test]
fn a_miss_through_an_alias_drops_the_other_class_copy_before_its_reload_returns() {
    let mut caches = Caches::new();
    caches.load_queued(0, at(LINE_A), false);
    let alias = Operand {
        ea: LINE_A + SET_STRIDE / 2,
        ..at(LINE_A)
    };
    assert_eq!(caches.load_queued(SETTLED, alias, false).0, Level::L2);
    assert_eq!(
        caches.load_queued(SETTLED + 1, at(LINE_A), false).0,
        Level::L2
    );
}
