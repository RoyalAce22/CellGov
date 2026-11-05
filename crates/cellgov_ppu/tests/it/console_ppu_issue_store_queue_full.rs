//! CellGov's PPU timing against the `ppu_issue_store_queue_full` console
//! capture.
//!
//! The probe times, with `mftb`, eleven runs each:
//! - 1, 2, 3, 4 and 6 back-to-back 128-byte `stmw r0` to separate lines,
//!   then a load of the first store's bytes or of a line no store writes;
//! - an `std` to line A and 7 to 31 more `std` to other 16-byte blocks,
//!   then the same loads;
//! - an `std` to line A and 15 or 16 `stmw r28` or 8 `stmw r24`, then the
//!   same loads.
//!
//! The replay holds each loop's cycles an iteration no farther from the
//! fastest run than the slowest run is, except the loops `RESIDUALS`
//! names.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_store_queue_full/ps3/cech20-cex-493/observation.json";

/// The loops the probe times.
const LOOP_COUNT: usize = 21;

/// log2 of each loop's iterations on the console, from the probe's
/// header.
const ITERATIONS_LOG2: u32 = 16;

/// The payload's word 44 when the probe wrote every word.
const DONE: u64 = 0x5053_5146;

/// The runs the probe takes of each loop.
const REPS: u64 = 11;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 8 * (2 * LOOP_COUNT + 3));
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(
        words[2 * LOOP_COUNT + 2],
        DONE,
        "the probe wrote every word"
    );
    assert_eq!(words[2 * LOOP_COUNT + 1], REPS, "the runs a loop");
    words
}

/// Loop `k`'s fastest and slowest runs, in core cycles an iteration.
fn console_cycles(words: &[u64], k: usize) -> (f64, f64) {
    let per = |w: u64| (w * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64;
    (per(words[2 * k]), per(words[2 * k + 1]))
}

/// `stmw r0,0(r7)`.
const STMW_R0: u32 = 0xbc07_0000;
/// `stmw r28,0(r7)`.
const STMW_R28: u32 = 0xbf87_0000;
/// `stmw r24,0(r7)`.
const STMW_R24: u32 = 0xbf07_0000;
/// `std r9,0(r7)`: the store of the chained doubleword to line A.
const STD_A: u32 = 0xf927_0000;
/// `std r10,0(r7)`.
const STD_R10: u32 = 0xf947_0000;
/// `ld r9,0(r7)`: line A.
const LD_A: u32 = 0xe927_0000;
/// `ld r9,1408(r7)`: line H, which no store writes.
const LD_H: u32 = 0xe927_0580;

/// A loop body: `lead` when it is not zero, then `op` with displacements
/// `first`, `first + step` and on, then `load` and the `bdnz` back to the
/// first word.
const fn body<const N: usize>(lead: u32, op: u32, first: u32, step: u32, load: u32) -> [u32; N] {
    let mut words = [0; N];
    let mut i = 0;
    if lead != 0 {
        words[0] = lead;
        i = 1;
    }
    let mut n = 0;
    while i < N - 2 {
        words[i] = op | (first + step * n);
        n += 1;
        i += 1;
    }
    words[N - 2] = load;
    // bdnz with a word displacement back to the loop's first word.
    words[N - 1] = 0x4200_0000 | (0x1_0000 - 4 * (N as u32 - 1));
    words
}

/// `N - 2` back-to-back `stmw r0` to lines A, B and on, then `load`.
const fn stmw_lines<const N: usize>(load: u32) -> [u32; N] {
    body(0, STMW_R0, 0, 128, load)
}

/// The `std` to line A, then `op` to the blocks from offset 128 in steps
/// of `step`, then `load`.
const fn after_std_a<const N: usize>(op: u32, step: u32, load: u32) -> [u32; N] {
    body(STD_A, op, 128, step, load)
}

const LOOP_0: [u32; 3] = stmw_lines(LD_A);
const LOOP_1: [u32; 4] = stmw_lines(LD_A);
const LOOP_2: [u32; 5] = stmw_lines(LD_A);
const LOOP_3: [u32; 6] = stmw_lines(LD_A);
const LOOP_4: [u32; 8] = stmw_lines(LD_A);
const LOOP_5: [u32; 3] = stmw_lines(LD_H);
const LOOP_6: [u32; 4] = stmw_lines(LD_H);
const LOOP_7: [u32; 5] = stmw_lines(LD_H);
const LOOP_8: [u32; 6] = stmw_lines(LD_H);
const LOOP_9: [u32; 8] = stmw_lines(LD_H);
const LOOP_10: [u32; 10] = after_std_a(STD_R10, 16, LD_A);
const LOOP_11: [u32; 18] = after_std_a(STD_R10, 16, LD_A);
const LOOP_12: [u32; 19] = after_std_a(STD_R10, 16, LD_A);
const LOOP_13: [u32; 26] = after_std_a(STD_R10, 16, LD_A);
const LOOP_14: [u32; 34] = after_std_a(STD_R10, 16, LD_A);
const LOOP_15: [u32; 18] = after_std_a(STD_R10, 16, LD_H);
const LOOP_16: [u32; 34] = after_std_a(STD_R10, 16, LD_H);
const LOOP_17: [u32; 18] = after_std_a(STMW_R28, 16, LD_A);
const LOOP_18: [u32; 19] = after_std_a(STMW_R28, 16, LD_A);
const LOOP_19: [u32; 11] = after_std_a(STMW_R24, 32, LD_A);
const LOOP_20: [u32; 11] = after_std_a(STMW_R24, 32, LD_H);

/// Each loop's body as the built probe lays it out, with r7 line A.
const LOOPS: [&[u32]; LOOP_COUNT] = [
    &LOOP_0, &LOOP_1, &LOOP_2, &LOOP_3, &LOOP_4, &LOOP_5, &LOOP_6, &LOOP_7, &LOOP_8, &LOOP_9,
    &LOOP_10, &LOOP_11, &LOOP_12, &LOOP_13, &LOOP_14, &LOOP_15, &LOOP_16, &LOOP_17, &LOOP_18,
    &LOOP_19, &LOOP_20,
];

/// Each loop replayed alone, in cycles an iteration.
fn cellgov_cycles() -> Vec<f64> {
    let seeds: &'static [Seed] = &[Seed::Gpr(7, ONE_LINE)];
    LOOPS
        .iter()
        .enumerate()
        .map(|(k, words)| {
            let r = Replay {
                loop_index: k,
                words,
                seeds,
            };
            replay(&r, 0).expect("the loop replays")
        })
        .collect()
}

/// The loops the model times outside the console's spread, each with the
/// cycles an iteration it may stay apart by.
///
/// - Loop 9 runs six 32-word `stmw r0`. The console takes about 73 cycles
///   a store multiple. The model takes 76 with the hold that fits the
///   short `stmw` loops of the `ppu_issue_stmw_words` capture.
/// - Loops 13, 14 and 16 run 24 and 32 `std`. Past 16 entries each store
///   costs the console about 13 cycles; the model drains a full queue at
///   the rate of one that is not full, about 3 cycles a store.
const RESIDUALS: [(usize, f64); 4] = [(9, 18.0), (13, 76.0), (14, 160.0), (16, 160.0)];

#[test]
fn the_issue_model_times_each_store_queue_full_loop_within_the_console_s_spread() {
    let w = console_words();
    let cycles = cellgov_cycles();
    assert_eq!(cycles.len(), LOOPS.len());
    let rows: Vec<_> = cycles
        .iter()
        .enumerate()
        .map(|(k, &cellgov)| {
            let (fastest, slowest) = console_cycles(&w, k);
            (k, cellgov, fastest, slowest - fastest)
        })
        .collect();
    for &(k, cellgov, fastest, spread) in &rows {
        let within = (cellgov - fastest).abs() <= spread;
        match RESIDUALS.iter().find(|r| r.0 == k) {
            Some(&(_, apart)) => {
                assert!(
                    !within,
                    "loop {k} now lands within the console's spread; drop it from \
                     the residuals: {rows:?}",
                );
                assert!(
                    (cellgov - fastest).abs() < apart,
                    "loop {k}: CellGov {cellgov:.3} cycles an iteration, the console's \
                     fastest {fastest:.3}, more than {apart} apart: {rows:?}",
                );
            }
            None => assert!(
                within,
                "loop {k}: CellGov {cellgov:.3} cycles an iteration, the console's \
                 fastest {fastest:.3} with a spread of {spread:.3}: {rows:?}",
            ),
        }
    }
}

#[test]
fn a_load_of_the_first_store_s_bytes_after_a_full_queue_still_waits_for_it() {
    let w = console_words();
    let cycles = cellgov_cycles();
    // Loop 11 runs 16 stores before the load of line A, loop 12 runs 17.
    let (fastest_16, _) = console_cycles(&w, 11);
    let (fastest_17, _) = console_cycles(&w, 12);
    assert!(
        fastest_17 >= fastest_16,
        "the console's load waits for its store past a full queue: \
         {fastest_16:.3} with 16 stores, {fastest_17:.3} with 17",
    );
    assert!(
        cycles[12] >= cycles[11],
        "the load of the first store's bytes waits for nothing once a 17th \
         store follows it: CellGov {:.3} with 16 stores, {:.3} with 17; the \
         console {fastest_16:.3} and {fastest_17:.3}",
        cycles[11],
        cycles[12],
    );
}
