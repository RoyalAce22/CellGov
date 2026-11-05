//! CellGov's PPU timing against the `ppu_issue_store_queue_drain` console
//! capture.
//!
//! The probe times, with `mftb`, eleven runs each of an `std` to line A,
//! more `std` to other addresses and a load of a line no store writes:
//! - 16 to 64 stores, one to each 16-byte block in turn;
//! - 32 stores to the same 16 blocks twice, two to a block, and one to a
//!   line;
//! - 32 or 16 stores with one or three independent `addi` after each.
//!
//! It also times the 96 `addi` alone. The replay holds each loop's cycles
//! an iteration no farther from the fastest run than the slowest run is,
//! except the loops `RESIDUALS` names.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_store_queue_drain/ps3/cech20-cex-493/observation.json";

/// The loops the probe times.
const LOOP_COUNT: usize = 17;

/// log2 of each loop's iterations on the console, from the probe's
/// header.
const ITERATIONS_LOG2: u32 = 15;

/// The payload's word 36 when the probe wrote every word.
const DONE: u64 = 0x5053_5144;

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

/// `std r9,0(r7)`: the store of the chained doubleword to line A.
const STD_A: u32 = 0xf927_0000;
/// `std r10,0(r7)`.
const STD_R10: u32 = 0xf947_0000;
/// `ld r9,4224(r7)`: line H, which no store writes.
const LD_H: u32 = 0xe927_1080;
/// `addi r16,r16,1`, `addi r17,r17,1` and `addi r18,r18,1`.
const ADDI: [u32; 3] = [0x3a10_0001, 0x3a31_0001, 0x3a52_0001];

/// Where a loop's stores after the one to line A go.
#[derive(Clone, Copy)]
enum Place {
    /// One to each 16-byte block from offset 128.
    Blocks,
    /// The 15 blocks from offset 128, then the first of them again and
    /// the same 15 blocks once more.
    Twice,
    /// Two to each 16-byte block from offset 128.
    Doublewords,
    /// One to each 128-byte line from offset 128.
    Lines,
}

/// The offset of store `i` after the one to line A.
const fn offset(place: Place, i: u32) -> u32 {
    match place {
        Place::Blocks => 128 + 16 * i,
        Place::Twice if i < 15 => 128 + 16 * i,
        Place::Twice if i == 15 => 128,
        Place::Twice => 128 + 16 * (i - 16),
        Place::Doublewords => 128 + 8 * i,
        Place::Lines => 128 * (i + 1),
    }
}

/// A loop body: the `std` to line A and `N` words in all, with `pad`
/// `addi` after each store, the other stores at `place`, then the load
/// of line H and the `bdnz` back to the first word.
const fn body<const N: usize>(place: Place, pad: usize) -> [u32; N] {
    let mut words = [0; N];
    let mut i = 0;
    let mut store = 0;
    while i < N - 2 {
        let at = i % (pad + 1);
        words[i] = if at != 0 {
            ADDI[at - 1]
        } else if i == 0 {
            STD_A
        } else {
            store += 1;
            STD_R10 | offset(place, store - 1)
        };
        i += 1;
    }
    words[N - 2] = LD_H;
    // bdnz with a word displacement back to the loop's first word.
    words[N - 1] = 0x4200_0000 | (0x1_0000 - 4 * (N as u32 - 1));
    words
}

/// The 96 `addi` of loop 14 alone, and the `bdnz`.
const fn addi_only<const N: usize>() -> [u32; N] {
    let mut words = [0; N];
    let mut i = 0;
    while i < N - 1 {
        words[i] = ADDI[i % 3];
        i += 1;
    }
    words[N - 1] = 0x4200_0000 | (0x1_0000 - 4 * (N as u32 - 1));
    words
}

const LOOP_0: [u32; 18] = body(Place::Blocks, 0);
const LOOP_1: [u32; 20] = body(Place::Blocks, 0);
const LOOP_2: [u32; 22] = body(Place::Blocks, 0);
const LOOP_3: [u32; 24] = body(Place::Blocks, 0);
const LOOP_4: [u32; 26] = body(Place::Blocks, 0);
const LOOP_5: [u32; 28] = body(Place::Blocks, 0);
const LOOP_6: [u32; 30] = body(Place::Blocks, 0);
const LOOP_7: [u32; 32] = body(Place::Blocks, 0);
const LOOP_8: [u32; 34] = body(Place::Blocks, 0);
const LOOP_9: [u32; 66] = body(Place::Blocks, 0);
const LOOP_10: [u32; 34] = body(Place::Twice, 0);
const LOOP_11: [u32; 34] = body(Place::Doublewords, 0);
const LOOP_12: [u32; 34] = body(Place::Lines, 0);
const LOOP_13: [u32; 66] = body(Place::Blocks, 1);
const LOOP_14: [u32; 130] = body(Place::Blocks, 3);
const LOOP_15: [u32; 66] = body(Place::Blocks, 3);
const LOOP_16: [u32; 97] = addi_only();

/// Each loop's body as the built probe lays it out, with r7 line A.
const LOOPS: [&[u32]; LOOP_COUNT] = [
    &LOOP_0, &LOOP_1, &LOOP_2, &LOOP_3, &LOOP_4, &LOOP_5, &LOOP_6, &LOOP_7, &LOOP_8, &LOOP_9,
    &LOOP_10, &LOOP_11, &LOOP_12, &LOOP_13, &LOOP_14, &LOOP_15, &LOOP_16,
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
/// - Loops 2 to 9 and 12 to 14 run more stores than the queue holds. The
///   console then drains about one store every 8 cycles, or one every 14
///   when each store is to a new line. The model drains a full queue at
///   the rate of one that is not full, about 3 cycles a store.
/// - Loop 11 runs two stores to each block. The model gathers each pair
///   into one entry, and the console takes 37 cycles more than the model
///   gives.
const RESIDUALS: [(usize, f64); 12] = [
    (2, 20.0),
    (3, 67.0),
    (4, 77.0),
    (5, 143.0),
    (6, 154.0),
    (7, 151.0),
    (8, 156.0),
    (9, 309.0),
    (11, 38.0),
    (12, 354.0),
    (13, 156.0),
    (14, 124.0),
];

#[test]
fn the_issue_model_times_each_store_queue_drain_loop_within_the_console_s_spread() {
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
fn stores_two_to_a_block_cost_less_than_stores_one_to_a_block() {
    let w = console_words();
    let cycles = cellgov_cycles();
    // Loop 11 runs 32 stores two to a block, loop 8 one to a block.
    let (two, _) = console_cycles(&w, 11);
    let (one, _) = console_cycles(&w, 8);
    assert!(
        two < one,
        "the console gathers: {two:.3} two to a block, {one:.3} one to a block"
    );
    assert!(
        cycles[11] < cycles[8],
        "CellGov {:.3} two to a block, {:.3} one to a block; the console {two:.3} \
         and {one:.3}",
        cycles[11],
        cycles[8],
    );
}
