//! The `ppu_lv2_call_preempted_forms` console capture: a loop of the
//! cheapest call beside a higher-priority waiter that a spinner wakes, and
//! two event flag handoffs, one that a set answers and one that a cancel
//! answers.
//!
//! `lv2_call_contended_microtest` in the CLI holds CellGov's run of the
//! probe through the runtime to this capture. These tests hold the
//! capture's values that the ABI constants' documents quote.

use cellgov_ps3_abi::hw::ppu_issue::{
    PPU_CYCLES_PER_TB_TICK, PPU_SYSCALL_CANCELLED_ROUND_TRIPS, PPU_SYSCALL_WOKEN_ROUND_TRIPS,
};
use cellgov_ps3_abi::lv2::syscall as sc;

const CONSOLE: &str =
    "../../tests/micro/ppu_lv2_call_preempted_forms/ps3/cech20-cex-493/observation.json";

/// The payload's word 90 when the probe wrote every word.
const DONE: u64 = 0x5050_4C50;

/// Runs a loop makes.
const REPS: usize = 5;

/// Handoffs a run of loops 3 and 4 makes.
const HANDOFFS: u64 = 1 << 12;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 768);
    let w: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(w[90], DONE, "the probe wrote every word");
    assert_eq!(w[76], 0, "every setup and teardown step returned 0");
    for word in (44..49).chain(52..59).chain(60..70) {
        assert_eq!(w[word], 0, "word {word}");
    }
    w
}

#[test]
fn each_wake_of_a_higher_priority_thread_costs_the_loop_21321_to_24514_cycles() {
    let w = console_words();
    let quiet = w[8..8 + REPS].iter().copied().min().expect("five runs");
    let lost: Vec<u64> = (0..REPS)
        .map(|r| {
            assert_eq!(w[32 + r], 0, "no wake in loop 1's run {r}");
            let cycles = (w[16 + r] - quiet) * PPU_CYCLES_PER_TB_TICK;
            (cycles as f64 / w[24 + r] as f64).round() as u64
        })
        .collect();
    assert_eq!(lost.iter().min(), Some(&21321), "{lost:?}");
    assert_eq!(lost.iter().max(), Some(&24514), "{lost:?}");
}

#[test]
fn a_handoff_answered_by_a_cancel_takes_4725_cycles_over_one_answered_by_a_set() {
    let w = console_words();
    let round =
        |at: usize| ((w[at] * PPU_CYCLES_PER_TB_TICK) as f64 / HANDOFFS as f64).round() as u64;
    assert_eq!(round(40), 61129, "the set handoff's fastest round");
    assert_eq!(round(42), 65854, "the cancel handoff's fastest round");
    let woken = |n: u64| PPU_SYSCALL_WOKEN_ROUND_TRIPS.iter().find(|&&(m, _)| m == n);
    let cancelled = |n: u64| {
        PPU_SYSCALL_CANCELLED_ROUND_TRIPS
            .iter()
            .find(|&&(m, _)| m == n)
    };
    let (Some(&(_, set)), Some(&(_, cancel))) =
        (woken(sc::EVENT_FLAG_WAIT), cancelled(sc::EVENT_FLAG_WAIT))
    else {
        panic!("both trips of an event flag wait are in the tables");
    };
    assert!(
        cancel > set,
        "a cancelled wait's trip {cancel} over a woken one's {set}"
    );
}
