//! A touch that the PPE cancels leaves the caches as they were.
//!
//! [CBE-Handbook p:145 s:6.1.6.2] A touch that the PPE cancels does not reload the cache.

use super::{Caches, Operand};
use cellgov_ps3_abi::hw::ppu_issue as k;

/// A line no test loads before it starts.
const LINE_A: u64 = 0x100_0000;
/// Another line, in another L1 set.
const LINE_B: u64 = LINE_A + 0x1000 + k::PPU_CACHE_LINE_BYTES;

#[test]
fn a_cancelled_touch_fills_no_line_and_records_no_arrival() {
    let mut caches = Caches::new();
    caches.touch(10, LINE_A, None);
    assert_eq!(caches.holds(LINE_A, LINE_A), (false, false));
    assert_eq!(caches.arrival(11, Operand::untranslated(LINE_A, 8)), 0);
}

#[test]
fn a_cancelled_touch_sends_no_request_ahead_of_the_next_touch() {
    let mut after_cancel = Caches::new();
    after_cancel.touch(10, LINE_A, None);
    assert_eq!(after_cancel.touch_free(), 0);
    after_cancel.touch(10, LINE_B, Some(LINE_B));
    let mut alone = Caches::new();
    alone.touch(10, LINE_B, Some(LINE_B));
    let at = Operand::untranslated(LINE_B, 8);
    assert_eq!(after_cancel.arrival(11, at), alone.arrival(11, at));
    assert_eq!(
        after_cancel.arrival(11, at),
        10 + k::PPU_MEMORY_LOAD_USE_CYCLES
    );
}
