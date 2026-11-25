//! CellGov's PPU timing against loops 0 to 6 of the
//! `ppu_issue_reload_return` console capture. Each loop touches a line
//! in memory, then 0 to 32 lines in memory in other L1 sets, then loads
//! a line that the L1 holds, and then waits long enough for each touch
//! to send its request.
//!
//! Each touch sends its request one touch interval after the touch
//! before it, and the thread waits for that turn. So each touch adds
//! about one touch interval to the pass, beyond the wait.

use crate::console_ppu_issue_reload_return::{
    cellgov_cycles_for, console_cycles, console_words, OTHER_TOUCHES,
};

/// Iterations each replay times, after as many that warm it. Every
/// touch misses the L2, so the loop repeats the same pass each
/// iteration; 512 iterations take the test past its time budget.
const ITERATIONS: u64 = 128;

/// The largest share of the cycles that one touch adds to the console's
/// pass that the model may differ by.
const PER_TOUCH_GAP: f64 = 0.10;

#[test]
fn each_touch_of_a_memory_line_adds_its_turn_to_the_pass_within_ten_percent_of_the_console() {
    let w = console_words();
    assert_eq!(OTHER_TOUCHES[0], 0, "loop 0 has no other touch");
    let (cellgov_none, console_none) = (cellgov_cycles_for(0, ITERATIONS), console_cycles(&w, 0));
    for (k, &n) in OTHER_TOUCHES.iter().enumerate().skip(1) {
        let cellgov = (cellgov_cycles_for(k, ITERATIONS) - cellgov_none) / n as f64;
        let console = (console_cycles(&w, k) - console_none) / n as f64;
        assert!(
            (console - cellgov).abs() / console <= PER_TOUCH_GAP,
            "loop {k}: each of {n} touches adds {cellgov:.2} cycles in CellGov and \
             {console:.2} on the console, more than {PER_TOUCH_GAP} of it apart",
        );
    }
}
