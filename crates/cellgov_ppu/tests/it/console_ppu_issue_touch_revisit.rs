//! CellGov's PPU timing against loop 3 of the `ppu_issue_touch_reload`
//! console capture: 8 touches of other lines in the L1 set of the load's
//! line, then the load. Each pass through a set touches the same 8 lines.
//!
//! The touches check the L1 before their reloads return, so the lines
//! from the last pass stay until then. The L1 keeps 3 of the 8 lines
//! from one pass to the next, and 5 touches reload.

use crate::console_ppu_issue_touch_reload::{
    assert_within, cellgov_cycles, console_cycles, console_words, IN_ORDER_CHASE_GAP,
};

/// The loop that touches the same 8 lines on each pass through a set.
const REVISIT: usize = 3;

/// [CBE-Handbook p:137 s:6.1.3.7] The DCache tag array is updated after the reload is written to the DCache.
#[test]
fn eight_touches_that_revisit_the_same_lines_of_the_loads_l1_set_delay_it_within_the_chase_spread()
{
    let w = console_words();
    assert_within(
        REVISIT,
        cellgov_cycles(REVISIT),
        console_cycles(&w, REVISIT),
        IN_ORDER_CHASE_GAP,
    );
}
