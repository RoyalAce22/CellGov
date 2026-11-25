//! CellGov's run of the `ppu_lv2_call_wait_ends` microtest, held to its
//! console capture.
//!
//! The header of the probe's `ppu/main.c` lists its loops and payload
//! words. The test compares three of them:
//!
//! - each wait that ends by a timeout of 6 us or more
//! - the ticks from a forced destroy to the return of the receive it ends
//! - the cycles each higher-priority wake costs a user-mode loop

use super::{cellgov_words, console_words, PPU_CYCLES_PER_TB_TICK};

const PROBE: &str = "ppu_lv2_call_wait_ends";

/// The payload's word 127 when the probe wrote every word.
const DONE: u64 = 0x5050_4C54;

/// Runs of each user-mode loop.
const REPS: usize = 5;

/// Rounds of the timed calls.
const ROUNDS: usize = 16;

/// Timeouts of each timed wait.
const TIMEOUTS: usize = 7;

/// The index of the 6 us timeout, the shortest the test compares.
const SIX_US: usize = 3;

/// Timed calls a round makes.
const TIMED: usize = 1 + 3 * TIMEOUTS + 4;

/// Rounds of the forced-destroy handoff.
const HANDOFF_ROUNDS: u64 = 5 << 10;

/// Check the words every setup and every loop of `w` returned.
fn check(side: &str, w: &[u64]) {
    assert_eq!(w[127], DONE, "{side}: the probe wrote every word");
    assert_eq!(w[96], 0, "{side}: every setup and teardown step returned 0");
    for word in (40..40 + TIMED).chain(56..62).chain(64..74).chain(76..78) {
        assert_eq!(w[word], 0, "{side}: word {word}");
    }
    assert_eq!(w[74], HANDOFF_ROUNDS, "{side}: the set helper's handoffs");
    assert_eq!(
        w[75], HANDOFF_ROUNDS,
        "{side}: the destroy helper's handoffs"
    );
    for r in 0..REPS {
        assert_eq!(w[32 + r], 0, "{side}: no wake in loop U1's run {r}");
        assert!(w[24 + r] > 0, "{side}: loop U2's run {r} saw wakes");
    }
}

/// Timed call `call`'s lower median sample, in core cycles.
fn median(w: &[u64], call: usize) -> u64 {
    let mut s: Vec<u64> = (0..ROUNDS)
        .map(|r| {
            let at = call * ROUNDS + r;
            let word = w[128 + at / 2];
            if at.is_multiple_of(2) {
                word >> 32
            } else {
                word & 0xFFFF_FFFF
            }
        })
        .collect();
    s.sort_unstable();
    s[(ROUNDS - 1) / 2] * PPU_CYCLES_PER_TB_TICK
}

/// The cycles each wake costs loop U2, one value for each of its runs.
fn lost_per_wake(w: &[u64]) -> Vec<f64> {
    let quiet = (8..8 + REPS).map(|i| w[i]).min().unwrap();
    (0..REPS)
        .map(|r| ((w[16 + r] - quiet) * PPU_CYCLES_PER_TB_TICK) as f64 / w[24 + r] as f64)
        .collect()
}

#[test]
fn each_wait_end_and_user_mode_wake_lies_within_the_console_s_spread() {
    let console = console_words(PROBE);
    let cellgov = cellgov_words(PROBE);
    check("console", &console);
    check("cellgov", &cellgov);
    let mut misses = Vec::new();
    for k in 0..3 {
        for t in SIX_US..TIMEOUTS {
            let call = 1 + k * TIMEOUTS + t;
            let (ours, theirs) = (median(&cellgov, call) as f64, median(&console, call) as f64);
            if (ours - theirs).abs() > 0.04 * theirs {
                misses.push(format!(
                    "timed call {call}: CellGov {ours:.0} cycles, the console {theirs:.0}"
                ));
            }
        }
    }
    let wake = |w: &[u64]| {
        let ended = HANDOFF_ROUNDS - w[78];
        (
            (w[92] * PPU_CYCLES_PER_TB_TICK) as f64 / ended as f64,
            (w[93] * PPU_CYCLES_PER_TB_TICK) as f64,
        )
    };
    let (theirs, least) = wake(&console);
    let (ours, _) = wake(&cellgov);
    if (ours - theirs).abs() > theirs - least {
        misses.push(format!(
            "a forced destroy's receive: CellGov {ours:.0} cycles, the console {theirs:.0}, \
             spread {:.0}",
            theirs - least
        ));
    }
    let spread = lost_per_wake(&console);
    let low = spread.iter().copied().fold(f64::MAX, f64::min);
    let high = spread.iter().copied().fold(0.0, f64::max);
    let mean = lost_per_wake(&cellgov).iter().sum::<f64>() / REPS as f64;
    if !(low..=high).contains(&mean) {
        misses.push(format!(
            "a user-mode wake costs {mean:.0} cycles in CellGov, {low:.0} to {high:.0} on the console"
        ));
    }
    assert!(misses.is_empty(), "{}", misses.join("\n"));
}
