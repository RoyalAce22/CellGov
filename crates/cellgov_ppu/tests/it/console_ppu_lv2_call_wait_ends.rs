//! Tests of the `ppu_lv2_call_wait_ends` console capture. The probe times:
//!
//! - waits that end by their timeout, each timed alone
//! - an event queue receive that a forced destroy ends
//! - a user-mode loop beside a higher-priority waiter that a spinner wakes
//!
//! `lv2_call_contended_microtest` in the CLI holds CellGov's run of the
//! probe through the runtime to this capture. These tests hold the
//! capture's values that the ABI constants' documents quote.

use cellgov_ps3_abi::hw::ppu_issue::{
    PPU_CYCLES_PER_TB_TICK, PPU_SYSCALL_CANCELLED_ROUND_TRIPS, PPU_SYSCALL_ROUND_TRIP_CYCLES,
    PPU_SYSCALL_TIMED_OUT_ROUND_TRIPS, PPU_SYSCALL_WOKEN_ROUND_TRIPS,
};
use cellgov_ps3_abi::lv2::syscall as sc;

const CONSOLE: &str =
    "../../tests/micro/ppu_lv2_call_wait_ends/ps3/cech20-cex-493/observation.json";

/// The payload's word 127 when the probe wrote every word.
const DONE: u64 = 0x5050_4C54;

/// Rounds of the timed calls.
const ROUNDS: usize = 16;

/// The timeouts of each timed wait, in microseconds.
const TIMEOUTS_US: [u64; 7] = [1, 2, 4, 6, 10, 100, 1000];

/// The timed waits, in the probe's order.
const WAITS: [u64; 3] = [
    sc::SEMAPHORE_WAIT,
    sc::EVENT_FLAG_WAIT,
    sc::EVENT_QUEUE_RECEIVE,
];

/// Timed calls a round makes.
const TIMED: usize = 1 + WAITS.len() * TIMEOUTS_US.len() + 4;

/// Rounds of the forced-destroy handoff.
const HANDOFF_ROUNDS: u64 = 5 << 10;

/// Core cycles in one microsecond of the time base.
const CYCLES_PER_US: u64 = 79_800_000 * PPU_CYCLES_PER_TB_TICK / 1_000_000;

/// The payload's header words, and its timed calls' lower median samples
/// in core cycles.
fn console() -> (Vec<u64>, Vec<u64>) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 1024 + 4 * TIMED * ROUNDS);
    let w: Vec<u64> = p[..1024]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(w[127], DONE, "the probe wrote every word");
    assert_eq!(w[96], 0, "every setup and teardown step returned 0");
    for word in (40..40 + TIMED).chain(56..62).chain(64..74).chain(76..78) {
        assert_eq!(w[word], 0, "word {word}");
    }
    let medians = p[1024..]
        .chunks_exact(4 * ROUNDS)
        .map(|call| {
            let mut s: Vec<u64> = call
                .chunks_exact(4)
                .map(|s| u64::from(u32::from_be_bytes(s.try_into().expect("4 bytes"))))
                .collect();
            s.sort_unstable();
            s[(s.len() - 1) / 2] * PPU_CYCLES_PER_TB_TICK
        })
        .collect();
    (w, medians)
}

/// The cycles the bracket adds beside the cheapest call's round trip.
fn rest(medians: &[u64]) -> u64 {
    medians[0] - PPU_SYSCALL_ROUND_TRIP_CYCLES
}

/// Wait `k`'s lower median sample at timeout `t`, less the timeout and
/// the bracket.
fn over(medians: &[u64], k: usize, t: usize) -> u64 {
    medians[1 + k * TIMEOUTS_US.len() + t] - TIMEOUTS_US[t] * CYCLES_PER_US - rest(medians)
}

#[test]
fn each_timed_out_trip_is_the_middle_of_its_wait_s_6_10_and_100_us_samples() {
    let (_, medians) = console();
    assert_eq!(rest(&medians), 351, "the bracket's own cycles");
    for (k, &number) in WAITS.iter().enumerate() {
        let mut middle = [3, 4, 5].map(|t| over(&medians, k, t));
        middle.sort_unstable();
        assert_eq!(
            PPU_SYSCALL_TIMED_OUT_ROUND_TRIPS
                .iter()
                .find(|&&(n, _)| n == number),
            Some(&(number, middle[1])),
            "syscall {number}",
        );
    }
}

#[test]
fn a_timeout_of_6_us_or_more_takes_within_four_percent_of_its_timeout_trip_and_bracket() {
    let (_, medians) = console();
    for (k, &(number, trip)) in PPU_SYSCALL_TIMED_OUT_ROUND_TRIPS.iter().enumerate() {
        assert_eq!(number, WAITS[k]);
        for t in 3..TIMEOUTS_US.len() {
            let console = medians[1 + k * TIMEOUTS_US.len() + t] as f64;
            let model = (TIMEOUTS_US[t] * CYCLES_PER_US + trip + rest(&medians)) as f64;
            assert!(
                (model - console).abs() <= 0.04 * console,
                "syscall {number} at {} us: CellGov {model:.0}, the console {console:.0}",
                TIMEOUTS_US[t],
            );
        }
    }
}

#[test]
fn a_timeout_of_2_us_or_less_returns_before_its_timeout_and_trip() {
    let (_, medians) = console();
    assert_eq!(medians[1], 14240, "a 1 us semaphore wait");
    for (k, &(number, trip)) in PPU_SYSCALL_TIMED_OUT_ROUND_TRIPS.iter().enumerate() {
        for t in 0..2 {
            let console = medians[1 + k * TIMEOUTS_US.len() + t];
            assert!(
                console < TIMEOUTS_US[t] * CYCLES_PER_US + trip,
                "syscall {number}: {console} cycles at {} us",
                TIMEOUTS_US[t],
            );
        }
    }
}

#[test]
fn a_4_us_timeout_returns_after_its_timeout_trip_and_bracket() {
    let (_, medians) = console();
    for (k, &(number, trip)) in PPU_SYSCALL_TIMED_OUT_ROUND_TRIPS.iter().enumerate() {
        let console = medians[1 + k * TIMEOUTS_US.len() + 2];
        let model = TIMEOUTS_US[2] * CYCLES_PER_US + trip + rest(&medians);
        assert!(
            console > model && console * 100 < model * 105,
            "syscall {number}: {console} cycles at 4 us, {model} in the model",
        );
    }
}

#[test]
fn a_receive_a_forced_destroy_ends_returns_29199_cycles_after_the_destroy_on_average() {
    let (w, _) = console();
    let ended = HANDOFF_ROUNDS - w[78];
    let mean = ((w[92] * PPU_CYCLES_PER_TB_TICK) as f64 / ended as f64).round() as u64;
    assert_eq!(mean, 29199);
    let entry = |table: &[(u64, u64)]| {
        table
            .iter()
            .find(|&&(n, _)| n == sc::EVENT_QUEUE_RECEIVE)
            .map(|&(_, trip)| trip)
    };
    let (Some(cancelled), Some(woken)) = (
        entry(&PPU_SYSCALL_CANCELLED_ROUND_TRIPS),
        entry(&PPU_SYSCALL_WOKEN_ROUND_TRIPS),
    ) else {
        panic!("both trips of an event queue receive are in the tables");
    };
    assert!(
        cancelled > woken,
        "a cancelled receive's trip {cancelled} over a woken one's {woken}"
    );
}
