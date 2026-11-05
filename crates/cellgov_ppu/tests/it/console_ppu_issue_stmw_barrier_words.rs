//! CellGov's PPU timing against the `ppu_issue_stmw_barrier_words`
//! console capture: an `lwsync` after a store multiple of 1 to 8 words,
//! and a `sync` after one of 1, 2, 4 and 8 words.
//!
//! The probe times fourteen loops with `mftb`, eleven runs each. Every
//! store addresses one data line, which r29 holds.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_stmw_barrier_words/ps3/cech20-cex-493/observation.json";

/// The loops the probe times.
const LOOPS: usize = 14;

/// The payload's word 30 when the probe wrote every word.
const DONE: u64 = 0x5050_5358;

/// The runs the probe takes of each loop.
const REPS: u64 = 11;

/// log2 of each loop's iterations on the console.
const ITERATIONS_LOG2: u32 = 16;

/// The payload of the capture at `path` as big-endian words.
fn words_at(path: &str) -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 8 * (2 * LOOPS + 3));
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[2 * LOOPS + 2], DONE, "the probe wrote every word");
    assert_eq!(
        (words[2 * LOOPS], words[2 * LOOPS + 1]),
        (REPS, u64::from(ITERATIONS_LOG2))
    );
    words
}

/// Loop `k`'s fastest and slowest runs, in core cycles an iteration.
fn console_cycles(words: &[u64], k: usize) -> (f64, f64) {
    let per = |t: u64| (t * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64;
    (per(words[2 * k]), per(words[2 * k + 1]))
}

/// `std r14, 0(r29)`.
const STD: u32 = 0xf9dd_0000;
const SYNC: u32 = 0x7c00_04ac;
const LWSYNC: u32 = 0x7c20_04ac;

/// `stmw rs, 0(r29)`: the 32 - `rs` words from `rs` to r31.
const fn stmw(rs: u32) -> u32 {
    0xbc1d_0000 | rs << 21
}

/// Each loop's store and barrier, in the probe's order.
const BODIES: [[u32; 2]; LOOPS] = [
    [STD, LWSYNC],
    [stmw(31), LWSYNC],
    [stmw(30), LWSYNC],
    [stmw(29), LWSYNC],
    [stmw(28), LWSYNC],
    [stmw(27), LWSYNC],
    [stmw(26), LWSYNC],
    [stmw(25), LWSYNC],
    [stmw(24), LWSYNC],
    [STD, SYNC],
    [stmw(31), SYNC],
    [stmw(30), SYNC],
    [stmw(28), SYNC],
    [stmw(24), SYNC],
];

/// CellGov's cycles an iteration for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let mut words = BODIES[k].to_vec();
    words.push(0x4200_0000 | ((4 * words.len() as u32).wrapping_neg() & 0xfffc));
    let r = Replay {
        loop_index: k,
        words: Box::leak(words.into_boxed_slice()),
        seeds: &[Seed::Gpr(29, ONE_LINE)],
    };
    replay(&r, 0).expect("the loop replays")
}

/// The share of a loop's fastest console run the model may lie on either
/// side of it, as for the `ppu_issue_barrier_forms` loops.
const BAND: f64 = 0.06;

/// The loops of an `stmw` of 1 to 8 words before an `lwsync`.
const LWSYNC_WORDS: std::ops::RangeInclusive<usize> = 1..=8;

#[test]
fn every_store_multiple_barrier_loop_lies_within_the_band() {
    let w = words_at(CONSOLE);
    let rows: Vec<(usize, f64, f64)> = (0..LOOPS)
        .map(|k| (k, cellgov_cycles(k), console_cycles(&w, k).0))
        .collect();
    for &(k, cellgov, console) in &rows {
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}: {rows:?}",
        );
    }
}

#[test]
fn each_word_of_a_store_multiple_adds_one_cycle_to_an_lwsync_loop() {
    let w = words_at(CONSOLE);
    let apart: Vec<f64> = LWSYNC_WORDS
        .map(|k| cellgov_cycles(k) - console_cycles(&w, k).0)
        .collect();
    let (low, high) = apart
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), &a| (lo.min(a), hi.max(a)));
    assert!(
        high - low < 1.0,
        "CellGov's distance from the console by word count: {apart:?}",
    );
    let per_word = (console_cycles(&w, 8).0 - console_cycles(&w, 1).0) / 7.0;
    assert!(
        (0.9..1.1).contains(&per_word),
        "the console adds {per_word:.2} cycles a word",
    );
}
