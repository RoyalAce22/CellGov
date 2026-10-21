//! CellGov's PPU timing against the `ppu_issue_multiple` console capture.
//!
//! The probe times `stmw` and `lmw` alone at 4, 8 and 16 words, the pair
//! on one line and on two, and an `stmw` followed by one load of its
//! first word, with `mftb`, five runs each. A load that reads bytes a
//! store multiple left in the store queue waits until the 16-byte entry
//! that holds them writes into the cache, and a recycled `lmw` runs its
//! microcode again. The pair on two lines pays no such wait. The replay
//! holds each loop's cycles an iteration no farther from the fastest run
//! than the slowest run is, except the loops `RESIDUALS` names.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_multiple/ps3/cech20-cex-493/observation.json";

/// log2 of each loop's iterations on the console, from the probe's
/// header.
const ITERATIONS_LOG2: [u32; 11] = [20, 20, 19, 20, 20, 19, 18, 18, 18, 17, 18];

/// The payload's word 24 when the probe wrote every word.
const DONE: u64 = 0x5050_534d;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 200);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[24], DONE, "the probe wrote every word");
    assert_eq!(words[23], 5, "five runs a loop");
    words
}

/// Loop `k`'s fastest and slowest runs, in core cycles an iteration.
fn console_cycles(words: &[u64], k: usize) -> (f64, f64) {
    let per = |w: u64| (w * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2[k]) as f64;
    (per(words[2 * k]), per(words[2 * k + 1]))
}

/// Each loop's body as the built probe lays it out, with r8 the first
/// line: `stmw` and `lmw` of r28, r24 and r16 at 0(r8), `lmw` at 128(r8)
/// for the second line, and `lwz r9,0(r8); add r10,r10,r9`.
const LOOPS: [&[u32]; 11] = [
    &[0xbf88_0000, 0x4200_fffc],
    &[0xbf08_0000, 0x4200_fffc],
    &[0xbe08_0000, 0x4200_fffc],
    &[0xbb88_0000, 0x4200_fffc],
    &[0xbb08_0000, 0x4200_fffc],
    &[0xba08_0000, 0x4200_fffc],
    &[0xbf08_0000, 0xbb08_0000, 0x4200_fff8],
    &[0xbf08_0000, 0xbb08_0080, 0x4200_fff8],
    &[0xbf88_0000, 0xbb88_0000, 0x4200_fff8],
    &[0xbe08_0000, 0xba08_0000, 0x4200_fff8],
    &[0xbf08_0000, 0x8128_0000, 0x7d4a_4a14, 0x4200_fff4],
];

/// The loops the model times outside the console's spread, each with the
/// cycles an iteration it may stay apart by.
///
/// Alone, `stmw` and `lmw` take longer on the console than the model's
/// handbook microword counts give, so the single loops and the pair on
/// two lines run short. The drain interval fits the 8- and 16-word pairs
/// to within 22 cycles either side: the
/// console's 8-word pair takes 53 cycles more than the 4-word one and its
/// 16-word pair only 35 more than the 8-word one, which no one interval
/// gives.
const RESIDUALS: [(usize, f64); 8] = [
    (0, 5.0),
    (1, 7.0),
    (2, 13.0),
    (4, 7.0),
    (5, 3.0),
    (6, 20.0),
    (7, 8.0),
    (9, 22.0),
];

#[test]
fn the_issue_model_times_each_multiple_loop_within_the_console_s_spread() {
    let w = console_words();
    let seeds: &'static [Seed] = &[Seed::Gpr(8, ONE_LINE)];
    let mut rows = Vec::new();
    for (k, words) in LOOPS.iter().enumerate() {
        let r = Replay {
            loop_index: k,
            words,
            seeds,
        };
        let cellgov = replay(&r, 0).expect("the loop replays");
        let (fastest, slowest) = console_cycles(&w, k);
        rows.push((k, cellgov, fastest, slowest - fastest));
    }
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
