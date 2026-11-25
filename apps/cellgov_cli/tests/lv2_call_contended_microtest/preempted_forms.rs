//! CellGov's run of the `ppu_lv2_call_preempted_forms` microtest, held to
//! its console capture.
//!
//! The probe times five loops. The header of its `ppu/main.c` lists them
//! and the payload words:
//!
//! 0. the cheapest call alone
//! 1. the cheapest call beside a spinner, with a higher-priority waiter
//!    that nothing wakes
//! 2. the same, with a spinner that wakes the waiter every 2000 time-base
//!    ticks; each wake takes the loop's hardware thread
//! 3. an event flag handoff that a set answers
//! 4. an event flag handoff that a cancel answers

use super::{cellgov_words, console_words, PPU_CYCLES_PER_TB_TICK, PPU_SYSCALL_ROUND_TRIP_CYCLES};

const PROBE: &str = "ppu_lv2_call_preempted_forms";

/// The payload's word 90 when the probe wrote every word.
const DONE: u64 = 0x5050_4C50;

/// Runs a loop makes.
const REPS: usize = 5;

/// Calls a run of loops 0 to 2 makes.
const CALLS: u64 = 1 << 13;

/// Handoffs a run of loops 3 and 4 makes.
const HANDOFFS: u64 = 1 << 12;

/// The first of four payload words: the fastest and slowest runs of loop
/// 3, then of loop 4.
const HANDOFF_WORDS: usize = 40;

/// Check the words every setup and every loop of `w` returned.
fn check(side: &str, w: &[u64]) {
    assert_eq!(w[90], DONE, "{side}: the probe wrote every word");
    assert_eq!(w[76], 0, "{side}: every setup and teardown step returned 0");
    for word in (44..49).chain(52..59).chain(60..70) {
        assert_eq!(w[word], 0, "{side}: word {word}");
    }
    assert_eq!(
        w[70],
        REPS as u64 * HANDOFFS,
        "{side}: the set helper's handoffs"
    );
    assert_eq!(
        w[71],
        REPS as u64 * HANDOFFS,
        "{side}: the cancel helper's handoffs"
    );
    for r in 0..REPS {
        assert_eq!(w[32 + r], 0, "{side}: no wake in loop 1's run {r}");
        assert!(w[24 + r] > 0, "{side}: loop 2's run {r} saw wakes");
    }
}

/// The cycles each wake costs loop 2, one value for each of its runs.
///
/// A run's value is its cycles above loop 1's fastest run, divided by
/// the run's wakes.
fn lost_per_wake(w: &[u64]) -> Vec<f64> {
    let quiet = (8..8 + REPS).map(|i| w[i]).min().unwrap();
    (0..REPS)
        .map(|r| ((w[16 + r] - quiet) * PPU_CYCLES_PER_TB_TICK) as f64 / w[24 + r] as f64)
        .collect()
}

/// The cycles each call of a loop of `w` costs beyond the round trip of
/// the cheapest call, from loop 0's fastest run.
fn rest(w: &[u64]) -> f64 {
    ((w[0] * PPU_CYCLES_PER_TB_TICK) as f64 / CALLS as f64).round()
        - PPU_SYSCALL_ROUND_TRIP_CYCLES as f64
}

/// Handoff `h`'s fastest and slowest runs, in cycles a round. Handoff 0
/// is loop 3, the set; handoff 1 is loop 4, the cancel.
fn handoff(w: &[u64], h: usize) -> (f64, f64) {
    let per = |ticks: u64| (ticks * PPU_CYCLES_PER_TB_TICK) as f64 / HANDOFFS as f64;
    let at = HANDOFF_WORDS + 2 * h;
    (per(w[at]), per(w[at + 1]))
}

#[test]
fn each_preempting_wake_and_cancelled_handoff_lies_within_the_console_s_spread() {
    let console = console_words(PROBE);
    let cellgov = cellgov_words(PROBE);
    check("console", &console);
    check("cellgov", &cellgov);
    let spread = lost_per_wake(&console);
    let low = spread.iter().copied().fold(f64::MAX, f64::min);
    let high = spread.iter().copied().fold(0.0, f64::max);
    let ours = lost_per_wake(&cellgov);
    let mean = ours.iter().sum::<f64>() / ours.len() as f64;
    assert!(
        (low..=high).contains(&mean),
        "a wake costs the loop {mean:.0} cycles in CellGov, {low:.0} to {high:.0} on the console",
    );
    for h in 0..2 {
        let (fastest, slowest) = handoff(&console, h);
        let console_round = fastest - rest(&console);
        let cellgov_round = handoff(&cellgov, h).0 - rest(&cellgov);
        assert!(
            (cellgov_round - console_round).abs() <= slowest - fastest,
            "handoff {h}: CellGov {cellgov_round:.0} cycles a round beside the console's \
             {console_round:.0}, spread {:.0}",
            slowest - fastest,
        );
    }
}
