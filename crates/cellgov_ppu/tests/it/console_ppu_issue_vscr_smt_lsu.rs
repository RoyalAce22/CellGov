//! CellGov's PPU timing against the `ppu_issue_vscr_smt_lsu` console
//! capture: whether one thread's `mtvscr` stalls the sibling thread's
//! loads and stores.
//!
//! The probe's sibling runs one loop of 32 instructions that loads a stop
//! flag and counts its rounds in memory, with no time-base read. Three
//! chains on one line run on the main thread beside each sibling.

use crate::console_ppu_issue_sibling_rate::{sibling_loop, COUNT, STOP};
use crate::console_ppu_issue_timing::{replay_beside, Replay, Seed, Sibling};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_vscr_smt_lsu/ps3/cech20-cex-493/cgov_frame.bin";

/// The payload's word 72 when the probe wrote every word.
const DONE: u64 = 0x5050_5753;

/// The thread exit value each sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The chains a phase times, and the siblings, phases 1 and 2.
const CHAINS: usize = 3;
const PHASES: usize = 2;

/// log2 of each chain's iterations on the console.
const ITERATIONS_LOG2: u32 = 16;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let words: Vec<u64> = frame[8..]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words.len(), 80);
    assert_eq!(words[72], DONE, "the probe wrote every word");
    assert_eq!(words[71], 5, "five runs a loop");
    for p in 1..=PHASES {
        assert_eq!(words[50 + p], 0, "phase {p}'s sibling was created");
        assert_eq!(words[55 + p], 0, "phase {p}'s sibling joined");
        assert_eq!(words[60 + p], SIBLING_EXIT, "phase {p}'s sibling exited");
        assert_eq!(words[65 + p], 1, "phase {p}'s sibling ran");
    }
    words
}

/// Chain `chain`'s mean run in `phase`, in core cycles an iteration.
///
/// Beside a sibling of `mtvscr` a run's cycles move with how the two
/// threads line up: the `lvx` chain runs 93 to 395 cycles an iteration.
/// The model gives a rate, which the mean of the five runs measures and
/// the fastest run does not.
fn console_mean(w: &[u64], phase: usize, chain: usize) -> f64 {
    let at = CHAINS * phase + chain;
    (w[30 + at] * PPU_CYCLES_PER_TB_TICK) as f64 / 5.0 / (1u64 << ITERATIONS_LOG2) as f64
}

/// `mtvscr v0`.
const MTVSCR: u32 = 0x1000_0644;
/// `addi rk, rk, 1`.
const fn addi(k: u32) -> u32 {
    0x3800_0001 | (k << 21) | (k << 16)
}
/// `lwz r(17 + k), 4k(r9)`.
fn lwz(k: u32) -> u32 {
    0x8009_0000 | ((17 + k) << 21) | (4 * k)
}
/// `lvx v(1 + k), 0, r9`.
fn lvx(k: u32) -> u32 {
    0x7c00_48ce | ((1 + k) << 21)
}
/// `stw r(17 + k), 4k(r9)`.
fn stw(k: u32) -> u32 {
    0x9009_0000 | ((17 + k) << 21) | (4 * k)
}

/// The line the chains load and store, in r9.
const LINE: u64 = 0x4000;
const LINE_SEED: &[Seed] = &[Seed::Gpr(9, LINE)];

/// The compiled sibling holds the stop flag's address in r10 and the
/// round count's in r8.
const SIBLING_SEEDS: &[Seed] = &[Seed::Gpr(10, STOP), Seed::Gpr(8, COUNT)];

/// Each phase's sibling, phases 1 and 2.
fn siblings() -> [Sibling; PHASES] {
    let leak = |w: [u32; 18]| -> &'static [u32] { Box::leak(Box::new(w)) };
    [
        Sibling {
            words: leak(sibling_loop([MTVSCR; 8], 10, 8)),
            seeds: SIBLING_SEEDS,
        },
        Sibling {
            words: leak(sibling_loop(
                [
                    addi(17),
                    addi(18),
                    addi(19),
                    addi(20),
                    addi(17),
                    addi(18),
                    addi(19),
                    addi(20),
                ],
                10,
                8,
            )),
            seeds: SIBLING_SEEDS,
        },
    ]
}

/// `body` closed by its `bdnz`.
fn looped(body: &[u32]) -> &'static [u32] {
    let mut words = body.to_vec();
    words.push(0x4200_0000 | ((4 * body.len() as u32).wrapping_neg() & 0xfffc));
    Box::leak(words.into_boxed_slice())
}

/// Each chain, in the probe's order: 8 `lwz`, 8 `lvx`, 8 `stw`.
fn chains() -> [Replay; CHAINS] {
    let eight = |f: fn(u32) -> u32| -> Vec<u32> { (0..8).map(|i| f(i % 4)).collect() };
    let chain = |k: usize, body: Vec<u32>| Replay {
        loop_index: k,
        words: looped(&body),
        seeds: LINE_SEED,
    };
    [
        chain(0, eight(lwz)),
        chain(1, eight(lvx)),
        chain(2, eight(stw)),
    ]
}

/// CellGov's cycles an iteration of chain `chain` beside the sibling of
/// `phase`.
fn cellgov_beside(phase: usize, chain: usize) -> f64 {
    replay_beside(&chains()[chain], 1, &siblings()[phase - 1]).expect("the chain replays")
}

/// The share of the console's mean run a held cell may lie on either side
/// of it: the `stw` chain beside the `mtvscr` sibling lies 16% above.
const BAND: f64 = 0.25;

/// Every cell, as (phase, chain): each chain beside the sibling of
/// back-to-back `mtvscr`, and beside the sibling of `addi`, which adds no
/// `mtvscr` stall.
const HELD: [(usize, usize); 6] = [(1, 0), (1, 1), (1, 2), (2, 0), (2, 1), (2, 2)];

#[test]
fn each_load_and_store_chain_beside_each_sibling_lies_within_the_band_of_the_mean_run() {
    let w = console_words();
    let rows: Vec<(usize, usize, f64, f64)> = HELD
        .iter()
        .map(|&(p, c)| (p, c, cellgov_beside(p, c), console_mean(&w, p, c)))
        .collect();
    for &(p, c, cellgov, console) in &rows {
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "phase {p} chain {c}: CellGov {cellgov:.2} cycles an iteration, the console's \
             mean {console:.2}, more than {BAND} of it apart: {rows:?}",
        );
    }
}
