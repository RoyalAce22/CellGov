//! CellGov's PPU timing against the `ppu_issue_chain` console capture.
//!
//! The probe times chains of dependent `add`, `mulli` and `mullw` with
//! `mftb`, five runs each. The issue model gives each chain its whole
//! latencies. On the console every chain takes a little more:
//!
//! - In a 256-long run between branches, 1.8% more for `add` and 0.8%
//!   for `mulli`, so the excess is per instruction and not only per
//!   iteration.
//! - In loops of 4 to 32, between 2.4% and 12.4% more, with no order in
//!   the chain's length: 16 dependent `add` lose 2.86 cycles an
//!   iteration and 32 lose 1.57.
//! - The 8-`add` loop's fastest run is 16.28 in the `ppu_issue_timing`
//!   capture and 16.51 in this one, farther apart than either capture's
//!   own runs spread.
//!
//! The handbook's issue rules give no fraction of a cycle, so ADR 0014
//! records the excess as an accepted gap with this capture as its
//! evidence. The replay holds each chain at or below the console's
//! fastest run, by no more than the gap measured here.

use crate::console_ppu_issue_timing::{replay, replay_beside, Replay, Seed, Sibling};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_chain/ps3/cech20-cex-493/observation.json";

/// log2 of each loop's iterations on the console, from the probe's
/// header.
const ITERATIONS_LOG2: [u32; 12] = [21, 20, 19, 18, 18, 17, 16, 17, 15, 12, 20, 17];

/// The payload's word 31 when the probe wrote every word.
const DONE: u64 = 0x5050_4348;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 256);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[31], DONE, "the probe wrote every word");
    assert_eq!(words[30], 5, "five runs a loop");
    assert_eq!(words[24], 0, "the sibling was created");
    assert_eq!(words[25], 1, "the sibling ran before loops 10 and 11");
    assert_eq!(words[27], 0, "the sibling joined");
    assert_eq!(words[28], 0x5157, "the sibling exited through its own path");
    words
}

/// Loop `k`'s fastest run, in core cycles an iteration.
fn console_cycles(words: &[u64], k: usize) -> f64 {
    (words[2 * k] * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2[k]) as f64
}

/// A loop of `n` copies of `word`, closed by its `bdnz`.
fn chain(word: u32, n: usize) -> &'static [u32] {
    let mut words = vec![word; n];
    words.push(0x4200_0000 | ((4 * n as u32).wrapping_neg() & 0xfffc));
    Box::leak(words.into_boxed_slice())
}

/// `add r9,r9,r15`.
const ADD: u32 = 0x7d29_7a14;
/// `mulli r9,r9,1`.
const MULLI: u32 = 0x1d29_0001;
/// `mullw r9,r9,r15`.
const MULLW: u32 = 0x7d29_79d6;

const ADD_SEEDS: &[Seed] = &[Seed::Gpr(9, 1), Seed::Gpr(15, 3)];
const MUL_SEEDS: &[Seed] = &[Seed::Gpr(9, 7), Seed::Gpr(15, 1)];

/// The largest share of a looped chain's console time the model may
/// fall short by: the 16-`mulli` loop's 12.4%.
const LOOP_GAP: f64 = 0.13;

/// The largest share of a 256-long chain's console time the model may
/// fall short by: the `add` run's 1.8%.
const RUN_GAP: f64 = 0.02;

/// The probe's sibling: one dependent `divd r15,r15,r16` and its `bdnz`.
const DIVD_SIBLING: Sibling = Sibling {
    words: &[0x7def_83d2, 0x4200_fffc],
    seeds: &[Seed::Gpr(15, 0x7fff_0000), Seed::Gpr(16, 1)],
};

/// The largest share of loop 10's fastest console run the model may sit
/// from it.
///
/// The `ppu_issue_shared` capture runs the same `add` chain beside a
/// sibling of eight dependent `divd` to a `bdnz`, and its runs take
/// between 20 and 183 cycles an iteration, so one console window does not
/// fix the stall between 103 and 164.
const SIBLING_GAP: f64 = 0.20;

/// Loop 11, the 8-`mulli` chain, is a residual: the model times it above
/// the console's slowest run, by at most this share of it.
///
/// Beside a sibling of dependent `divd` the console's cost of a `mulli`
/// chain moves from capture to capture: means of 197 and 154 in the
/// `ppu_issue_shared` and `ppu_issue_multiply` captures, and runs of 194
/// to 231 here. Beside independent `divd` or dependent `divw`, `fdiv` or
/// `fsqrt` its mean stays between 228 and 263. The model gives one cost
/// for every divide sibling, near the second set.
///
/// [CBE-Handbook p:301 s:10.2.3.1] the PPU can flush an instruction that
/// depends on a divide in progress, and its thread then waits at
/// dispatch until the divide completes.
const LOOP_11_ABOVE_SLOWEST: f64 = 0.10;

/// Loop `k`'s fastest and slowest console runs, and CellGov's cycles an
/// iteration beside the probe's sibling.
fn loop_beside_divd(w: &[u64], k: usize, word: u32, seeds: &'static [Seed]) -> (f64, f64, f64) {
    let r = Replay {
        loop_index: k,
        words: chain(word, 8),
        seeds,
    };
    let cellgov = replay_beside(&r, 1, &DIVD_SIBLING).expect("the chain replays");
    let per = |tb: u64| (tb * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2[k]) as f64;
    (per(w[2 * k]), per(w[2 * k + 1]), cellgov)
}

#[test]
fn the_issue_model_times_loop_10_beside_the_divd_sibling_near_the_console() {
    let w = console_words();
    let (fastest, slowest, cellgov) = loop_beside_divd(&w, 10, ADD, ADD_SEEDS);
    assert!(
        (fastest..=slowest).contains(&cellgov)
            || (cellgov - fastest).abs() / fastest <= SIBLING_GAP,
        "loop 10: CellGov {cellgov:.2} cycles an iteration, the console {fastest:.2} \
         to {slowest:.2}",
    );
}

#[test]
fn the_issue_model_times_loop_11_beside_the_divd_sibling_just_above_the_console() {
    let w = console_words();
    let (fastest, slowest, cellgov) = loop_beside_divd(&w, 11, MULLI, MUL_SEEDS);
    assert!(
        cellgov > slowest && (cellgov - fastest) / fastest > SIBLING_GAP,
        "loop 11 now lands near the console; drop the residual: CellGov {cellgov:.2}, \
         the console {fastest:.2} to {slowest:.2}",
    );
    assert!(
        (cellgov - slowest) / slowest <= LOOP_11_ABOVE_SLOWEST,
        "loop 11: CellGov {cellgov:.2} cycles an iteration, the console {fastest:.2} \
         to {slowest:.2}",
    );
}

#[test]
fn the_issue_model_times_each_chain_at_most_the_accepted_gap_below_the_console() {
    let w = console_words();
    // (loop, instruction, chain length, seeds, gap)
    let loops: [(usize, u32, usize, &'static [Seed], f64); 10] = [
        (0, ADD, 4, ADD_SEEDS, LOOP_GAP),
        (1, ADD, 8, ADD_SEEDS, LOOP_GAP),
        (2, ADD, 16, ADD_SEEDS, LOOP_GAP),
        (3, ADD, 32, ADD_SEEDS, LOOP_GAP),
        (4, MULLI, 4, MUL_SEEDS, LOOP_GAP),
        (5, MULLI, 8, MUL_SEEDS, LOOP_GAP),
        (6, MULLI, 16, MUL_SEEDS, LOOP_GAP),
        (7, MULLW, 8, MUL_SEEDS, LOOP_GAP),
        (8, ADD, 256, ADD_SEEDS, RUN_GAP),
        (9, MULLI, 256, MUL_SEEDS, RUN_GAP),
    ];
    let mut rows = Vec::new();
    for (k, word, n, seeds, gap) in loops {
        let r = Replay {
            loop_index: k,
            words: chain(word, n),
            seeds,
        };
        let cellgov = replay(&r, 0).expect("the chain replays");
        rows.push((k, cellgov, console_cycles(&w, k), gap));
    }
    for &(k, cellgov, fastest, gap) in &rows {
        assert!(
            cellgov <= fastest && (fastest - cellgov) / fastest <= gap,
            "loop {k}: CellGov {cellgov:.3} cycles an iteration, the console's \
             fastest {fastest:.3}, more than {gap} of it apart or above it: {rows:?}",
        );
    }
}
