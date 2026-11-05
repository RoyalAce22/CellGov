//! CellGov's PPU timing against the `ppu_issue_load_hit_store` console
//! capture.
//!
//! The probe times, with `mftb`, eleven runs each:
//! - an `std` and `ld` chain on line A, with stores to other lines
//!   before or after the store to A;
//! - a store multiple and load multiple pair, then independent `addi`;
//! - a load multiple alone, then independent `addi`;
//! - an `std` and `ld` pair, then independent `addi`.
//!
//! The replay holds each loop's cycles an iteration no farther from the
//! fastest run than the slowest run is, except the loops `RESIDUALS`
//! names.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_load_hit_store/ps3/cech20-cex-493/observation.json";

/// log2 of each loop's iterations on the console, from the probe's
/// header.
const ITERATIONS_LOG2: [u32; 12] = [18, 18, 18, 18, 18, 18, 17, 17, 17, 17, 18, 18];

/// The payload's word 26 when the probe wrote every word.
const DONE: u64 = 0x504c_4853;

/// The runs the probe takes of each loop.
const REPS: u64 = 11;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 216);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[26], DONE, "the probe wrote every word");
    assert_eq!(words[25], REPS, "the runs a loop");
    words
}

/// Loop `k`'s fastest and slowest runs, in core cycles an iteration.
fn console_cycles(words: &[u64], k: usize) -> (f64, f64) {
    let per = |w: u64| (w * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2[k]) as f64;
    (per(words[2 * k]), per(words[2 * k + 1]))
}

const STD_A: u32 = 0xf927_0000;
const STD_B: u32 = 0xf947_0080;
const STD_C: u32 = 0xf947_0100;
const STD_D: u32 = 0xf947_0180;
const STD_E: u32 = 0xf927_0200;
const LD_A: u32 = 0xe927_0000;
const STMW: u32 = 0xbf07_0000;
const LMW: u32 = 0xbb07_0000;

/// `addi r16,r16,1` through `addi r23,r23,1`.
const ADDI: [u32; 8] = [
    0x3a10_0001,
    0x3a31_0001,
    0x3a52_0001,
    0x3a73_0001,
    0x3a94_0001,
    0x3ab5_0001,
    0x3ad6_0001,
    0x3af7_0001,
];

/// `head`, then `N - head.len() - 1` of the `addi` in turn, then the
/// `bdnz` back to the first word.
const fn body<const N: usize>(head: &[u32]) -> [u32; N] {
    let mut words = [0; N];
    let mut i = 0;
    while i < N - 1 {
        words[i] = if i < head.len() {
            head[i]
        } else {
            ADDI[(i - head.len()) % 8]
        };
        i += 1;
    }
    // bdnz with a word displacement back to the loop's first word.
    words[N - 1] = 0x4200_0000 | (0x1_0000 - 4 * (N as u32 - 1));
    words
}

const LOOP_0: [u32; 3] = body(&[STD_A, LD_A]);
const LOOP_1: [u32; 4] = body(&[STD_A, STD_B, LD_A]);
const LOOP_2: [u32; 4] = body(&[STD_B, STD_A, LD_A]);
const LOOP_3: [u32; 6] = body(&[STD_A, STD_B, STD_C, STD_D, LD_A]);
const LOOP_4: [u32; 6] = body(&[STD_B, STD_C, STD_D, STD_A, LD_A]);
const LOOP_5: [u32; 6] = body(&[STD_B, STD_C, STD_D, STD_E, LD_A]);
const LOOP_6: [u32; 3] = body(&[STMW, LMW]);
const LOOP_7: [u32; 11] = body(&[STMW, LMW]);
const LOOP_8: [u32; 35] = body(&[STMW, LMW]);
const LOOP_9: [u32; 34] = body(&[LMW]);
const LOOP_10: [u32; 35] = body(&[STD_A, LD_A]);
const LOOP_11: [u32; 33] = body(&[]);

/// Each loop's body as the built probe lays it out, with r7 line A.
const LOOPS: [&[u32]; 12] = [
    &LOOP_0, &LOOP_1, &LOOP_2, &LOOP_3, &LOOP_4, &LOOP_5, &LOOP_6, &LOOP_7, &LOOP_8, &LOOP_9,
    &LOOP_10, &LOOP_11,
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
/// The pair alone runs short, as it does in the `ppu_issue_multiple`
/// capture. On the console the pair with 8 `addi` after it runs 29
/// cycles faster than the pair alone, and the model gives both the same
/// time.
const RESIDUALS: [(usize, f64); 2] = [(6, 18.0), (7, 13.0)];

#[test]
fn the_issue_model_times_each_load_hit_store_loop_within_the_console_s_spread() {
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
fn independent_addi_after_a_load_that_hits_a_store_wait_for_its_recycle() {
    let w = console_words();
    let cycles = cellgov_cycles();
    // Loop 10 is loop 0 with the 32 `addi` of loop 11 after its load.
    let added = |c: &dyn Fn(usize) -> f64| c(10) - c(0).max(c(11));
    let fastest = added(&|k| console_cycles(&w, k).0);
    let slowest = added(&|k| console_cycles(&w, k).1);
    let cellgov = added(&|k| cycles[k]);
    let (lo, hi) = (fastest.min(slowest), fastest.max(slowest));
    assert!(
        lo > 0.0,
        "the console hides the addi in the wait: {lo:.3}..{hi:.3}"
    );
    assert!(
        (lo..=hi).contains(&cellgov),
        "the addi add {cellgov:.3} cycles past the longer of the chain and the \
         addi alone; the console {lo:.3} to {hi:.3}",
    );
}
