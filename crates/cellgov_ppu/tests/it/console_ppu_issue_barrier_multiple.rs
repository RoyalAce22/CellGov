//! CellGov's PPU timing against the `ppu_issue_barrier_multiple` console
//! capture: what a barrier costs after a store multiple or a store
//! string, by the store's word count and by the work between the store
//! and the barrier.
//!
//! The probe times thirteen loops with `mftb`, five runs each. Every
//! store addresses one data line, which r29 holds.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::{PPU_CYCLES_PER_TB_TICK, PPU_STORE_MULTIPLE_EXTRA_CYCLES};

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_barrier_multiple/ps3/cech20-cex-493/observation.json";

/// The loops the probe times.
const LOOPS: usize = 13;

/// The payload's word 28 when the probe wrote every word.
const DONE: u64 = 0x5050_424d;

/// log2 of each loop's iterations on the console.
const ITERATIONS_LOG2: u32 = 16;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 8 * (2 * LOOPS + 3));
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[2 * LOOPS + 2], DONE, "the probe wrote every word");
    assert_eq!(
        (words[2 * LOOPS], words[2 * LOOPS + 1]),
        (5, u64::from(ITERATIONS_LOG2))
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
/// `std r14, 64(r29)`.
const STD64: u32 = 0xf9dd_0040;
/// `stmw r24, 0(r29)`: 8 words.
const STMW24: u32 = 0xbf1d_0000;
/// `stmw r28, 0(r29)`: 4 words, one 16-byte block.
const STMW28: u32 = 0xbf9d_0000;
/// `stmw r30, 0(r29)`: 2 words.
const STMW30: u32 = 0xbfdd_0000;
/// `stswi r24, r29, 8`.
const STSWI8: u32 = 0x7f1d_45aa;
/// `stswi r24, r29, 32`.
const STSWI32: u32 = 0x7f1d_05aa;
/// `sync`.
const SYNC: u32 = 0x7c00_04ac;
/// `lwsync`.
const LWSYNC: u32 = 0x7c20_04ac;
/// `addi rn, rn, 1` for r3 through r10.
const ADDI8: [u32; 8] = [
    0x3863_0001,
    0x3884_0001,
    0x38a5_0001,
    0x38c6_0001,
    0x38e7_0001,
    0x3908_0001,
    0x3929_0001,
    0x394a_0001,
];

/// Each loop's body before its `bdnz`, in the probe's order.
fn bodies() -> [Vec<u32>; LOOPS] {
    let addi = |n: usize| ADDI8.iter().copied().cycle().take(8 * n);
    [
        vec![STD, LWSYNC],
        vec![STMW24, LWSYNC],
        vec![STMW30, LWSYNC],
        vec![STMW28, LWSYNC],
        [STMW24]
            .into_iter()
            .chain(addi(1))
            .chain([LWSYNC])
            .collect(),
        [STMW24]
            .into_iter()
            .chain(addi(4))
            .chain([LWSYNC])
            .collect(),
        vec![STSWI8, LWSYNC],
        vec![STSWI32, LWSYNC],
        vec![STSWI32, SYNC],
        vec![STMW30, SYNC],
        vec![STMW24, STD64, LWSYNC],
        [STD].into_iter().chain(addi(1)).chain([LWSYNC]).collect(),
        vec![STD, SYNC],
    ]
}

/// CellGov's cycles an iteration for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let mut words = bodies()[k].clone();
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

/// The loops the model times outside the band, each with the share of
/// the console's fastest run it may stay apart by. The share is negative
/// when the model times the loop faster than the console.
///
/// - Loops 4, 5 and 11 run independent `addi` between the store and the
///   `lwsync`. On the console the `addi` run while the barrier waits for
///   the store, so 8 of them add at most about 5 cycles to the loop. The model
///   starts the barrier's wait when the barrier issues, so it adds the
///   `addi` in full.
/// - Loop 10 runs an `std` after the `stmw`. The console takes about 11
///   cycles more than the `stmw` loop alone, and the model about 1.
/// - Loop 2 runs a 2-word `stmw`. Of its five runs here, the fastest is
///   slower than the 4- and 8-word loops' fastest. The
///   `ppu_issue_stmw_barrier_words` capture takes 11 runs a loop. In it,
///   the 2-word loop lies between the 1- and 4-word loops and within the
///   band of the model.
const RESIDUALS: [(usize, f64); 5] = [(2, -0.09), (4, 0.16), (5, 0.14), (10, -0.10), (11, 0.12)];

#[test]
fn each_barrier_loop_lies_within_the_band_or_its_named_residual() {
    let w = console_words();
    let rows: Vec<(usize, f64, f64)> = (0..LOOPS)
        .map(|k| (k, cellgov_cycles(k), console_cycles(&w, k).0))
        .collect();
    for &(k, cellgov, console) in &rows {
        let signed = (cellgov - console) / console;
        let apart = signed.abs();
        match RESIDUALS.iter().find(|r| r.0 == k) {
            Some(&(_, bound)) => {
                assert!(
                    apart > BAND,
                    "loop {k} now lies within the band; drop it from the residuals: {rows:?}",
                );
                assert!(
                    signed * bound > 0.0 && apart < bound.abs(),
                    "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's \
                     fastest {console:.2}, not on the side of it {bound} names or more \
                     than {bound} of it apart",
                );
            }
            None => assert!(
                apart <= BAND,
                "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
                 {console:.2}: {rows:?}",
            ),
        }
    }
}

/// The `ppu_issue_barrier_forms` capture, whose loop 9 is this probe's
/// loop 1.
const BARRIER_FORMS: &str =
    "../../tests/micro/ppu_issue_barrier_forms/ps3/cech20-cex-493/observation.json";

/// The `stmw r24; lwsync` loop's fastest run in the `ppu_issue_barrier_forms`
/// capture, in core cycles an iteration.
fn barrier_forms_stmw_lwsync() -> f64 {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(BARRIER_FORMS))
        .unwrap_or_else(|e| panic!("{BARRIER_FORMS}: {e}"));
    let p = &observation.memory_regions[0].data;
    let at = 8 * 2 * 9;
    let ticks = u64::from_be_bytes(p[at..at + 8].try_into().expect("8 bytes"));
    (ticks * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64
}

#[test]
fn the_barrier_forms_capture_timed_the_8_word_stmw_lwsync_loop_slower_than_this_one() {
    let here = console_cycles(&console_words(), 1).0;
    let there = barrier_forms_stmw_lwsync();
    let cellgov = cellgov_cycles(1);
    assert!(
        there > here * (1.0 + 2.0 * BAND),
        "the two captures time stmw r24; lwsync at {here:.2} and {there:.2}",
    );
    assert!(
        (cellgov - here).abs() / here <= BAND,
        "CellGov {cellgov:.2}"
    );
    assert!((there - cellgov) / there > BAND, "CellGov {cellgov:.2}");
}

/// The loops of a store string before an `lwsync`: `stswi` of 8 and 32
/// bytes.
const STORE_STRING: [usize; 2] = [6, 7];

#[test]
fn a_store_string_before_a_barrier_takes_no_store_multiple_hold() {
    // An 8-word `stswi` before an `lwsync` costs the console less than an
    // 8-word `stmw`.
    let w = console_words();
    assert!(console_cycles(&w, 7).0 < console_cycles(&w, 1).0);
    assert!(cellgov_cycles(7) < cellgov_cycles(1));
    let hold = f64::from(PPU_STORE_MULTIPLE_EXTRA_CYCLES);
    for k in STORE_STRING {
        let (cellgov, console) = (cellgov_cycles(k), console_cycles(&w, k).0);
        assert!(
            (cellgov - console).abs() < hold / 2.0,
            "loop {k}: CellGov {cellgov:.2}, the console {console:.2}",
        );
    }
}
