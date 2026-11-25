//! A reload that has not written its line writes nothing when the line
//! leaves the caches before the reload returns, and a stall delays the
//! cycle it writes.

use super::{Caches, Level, Operand};
use cellgov_ps3_abi::hw::ppu_issue as k;

/// A line no test loads before it starts.
const LINE_A: u64 = 0x100_0000;
/// The bytes between two lines in one L1 set.
const SET_STRIDE: u64 = k::PPU_L1D_SETS as u64 * k::PPU_CACHE_LINE_BYTES;
/// The bytes between two lines in one L2 set.
const L2_SET_STRIDE: u64 = k::PPU_L2_SETS as u64 * k::PPU_CACHE_LINE_BYTES;
/// Cycles that outlast a reload from memory and the hold it puts on its
/// L1 set.
const SETTLED: u64 = k::PPU_MEMORY_LOAD_USE_CYCLES + k::PPU_LOAD_MISS_ENTRY_REUSE_CYCLES + 1;

fn at(ea: u64) -> Operand {
    Operand::untranslated(ea, 8)
}

/// Caches after a load of `LINE_A` at cycle 0, whose reload from memory
/// has not returned at cycle 1.
fn line_a_on_its_way() -> Caches {
    let mut caches = Caches::new();
    assert_eq!(caches.load_queued(0, at(LINE_A), false).0, Level::Memory);
    caches
}

/// [CBE-Handbook p:147 s:6.1.6.5] `dcbf` invalidates the line in all caches.
#[test]
fn a_flush_of_a_line_on_its_way_keeps_its_reload_out_of_the_l1() {
    let mut caches = line_a_on_its_way();
    caches.flush(1, LINE_A);
    assert_eq!(caches.holds(LINE_A, LINE_A), (false, false));
}

/// [CBE-Handbook p:147 s:6.1.6.4] When the line that `dcbz` addresses is present in the L1, the L1 invalidates it before the operation goes to the L2.
#[test]
fn a_zero_of_a_line_on_its_way_keeps_its_reload_out_of_the_l1() {
    let mut caches = line_a_on_its_way();
    caches.zero(1, LINE_A);
    assert_eq!(caches.holds(LINE_A, LINE_A), (false, true));
}

/// [CBE-Handbook p:138 s:6.1.3.10] A line found at the other congruence class is invalidated, and the data from the L2 is placed at the congruence class of the effective address.
#[test]
fn a_store_through_an_alias_keeps_the_other_class_reload_out_of_the_l1() {
    let mut caches = line_a_on_its_way();
    let alias = Operand {
        ea: LINE_A + SET_STRIDE / 2,
        ..at(LINE_A)
    };
    caches.store(1, alias);
    assert_eq!(caches.holds(LINE_A, LINE_A), (false, true));
}

/// [CBE-Handbook p:140 s:6.1.5.1] The L2 includes all of the L1 data cache contents.
#[test]
fn an_l2_eviction_of_a_line_on_its_way_keeps_its_reload_out_of_the_l1() {
    let mut caches = line_a_on_its_way();
    for n in 1..=k::PPU_L2_WAYS as u64 {
        caches.store(1, at(LINE_A + n * L2_SET_STRIDE));
    }
    assert_eq!(caches.holds(LINE_A, LINE_A), (false, false));
}

/// Line `i` of `LINE_A`'s L1 set.
const fn in_set(i: u64) -> u64 {
    LINE_A + i * SET_STRIDE
}

/// Where a load finds line 0 of `LINE_A`'s full L1 set, `SETTLED` cycles
/// after a touch of a fifth line in the set and a stall of `stalled`
/// cycles that begins at the touch.
fn line_0_after_a_stalled_touch(stalled: u64) -> Level {
    let mut caches = Caches::new();
    let mut t = 0;
    for i in 0..k::PPU_L1D_WAYS as u64 {
        caches.load_queued(t, at(in_set(i)), false);
        t += SETTLED;
    }
    caches.touch(t, in_set(4), Some(in_set(4)));
    caches.stretch_touches(t, stalled);
    caches.load_queued(t + SETTLED, at(in_set(0)), false).0
}

#[test]
fn a_stall_delays_the_cycle_a_reload_writes_its_line() {
    assert_eq!(line_0_after_a_stalled_touch(0), Level::L2);
    assert_eq!(line_0_after_a_stalled_touch(SETTLED), Level::L1);
}
