//! Where a store finds its lines, and what the caches hold after it.
//!
//! [CBE-Handbook p:139 s:6.1.5] the L2 allocates on a store miss.

use super::{Caches, Level, Operand};
use cellgov_ps3_abi::hw::ppu_issue as k;

/// A line no test loads before it starts.
const LINE_A: u64 = 0x100_0000;
/// The line after `LINE_A`.
const LINE_B: u64 = LINE_A + k::PPU_CACHE_LINE_BYTES;
/// The doubleword whose first 4 bytes end `LINE_A` and whose last 4
/// bytes start `LINE_B`.
const ACROSS: u64 = LINE_B - 4;

fn at(ea: u64) -> Operand {
    Operand::untranslated(ea, 8)
}

#[test]
fn a_store_to_a_line_the_l2_does_not_hold_finds_it_in_memory() {
    let mut caches = Caches::new();
    assert_eq!(caches.store(0, at(LINE_A)), Level::Memory);
}

#[test]
fn a_store_miss_leaves_its_line_in_the_l2_for_a_later_load() {
    let mut caches = Caches::new();
    caches.store(0, at(LINE_A));
    assert_eq!(caches.holds(LINE_A, LINE_A), (false, true));
    assert_eq!(caches.load(at(LINE_A), false), Level::L2);
}

#[test]
fn a_second_store_to_a_line_finds_it_in_the_l2() {
    let mut caches = Caches::new();
    caches.store(0, at(LINE_A));
    assert_eq!(caches.store(0, at(LINE_A)), Level::L2);
}

#[test]
fn a_store_across_two_lines_finds_memory_when_the_l2_lacks_either_line() {
    for held in [LINE_A, LINE_B] {
        let mut caches = Caches::new();
        caches.load(at(held), false);
        assert_eq!(
            caches.store(0, at(ACROSS)),
            Level::Memory,
            "with only {held:#x} in the L2"
        );
    }
    let mut caches = Caches::new();
    caches.load(at(LINE_A), false);
    caches.load(at(LINE_B), false);
    assert_eq!(caches.store(0, at(ACROSS)), Level::L2);
}
