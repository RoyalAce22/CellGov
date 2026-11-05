//! CellGov's PPU timing against the `ppu_issue_vscr_smt_drain` console
//! capture: whether one thread's `mtvscr`, while it waits for its own held
//! `vaddubm` work, stalls the sibling thread's loads and stores.
//!
//! The probe's sibling runs one loop of 32 instructions that loads a stop
//! flag and counts its rounds in memory, with no time-base read. Three
//! chains on one line run on the main thread beside it.

use crate::console_ppu_issue_sibling_rate::{sibling_loop, COUNT, STOP};
use crate::console_ppu_issue_timing::{replay_beside, Replay, Seed, Sibling};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_vscr_smt_drain/ps3/cech20-cex-493/cgov_frame.bin";

/// The payload's word 72 when the probe wrote every word.
const DONE: u64 = 0x5050_5753;

/// The thread exit value the sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The chains a phase times.
const CHAINS: usize = 3;

/// The phase that runs the sibling.
const PHASE: usize = 1;

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
    assert_eq!(words[50 + PHASE], 0, "the sibling was created");
    assert_eq!(words[55 + PHASE], 0, "the sibling joined");
    assert_eq!(words[60 + PHASE], SIBLING_EXIT, "the sibling exited");
    assert_eq!(words[65 + PHASE], 1, "the sibling ran");
    words
}

/// Chain `chain`'s mean run beside the sibling, in core cycles an
/// iteration.
fn console_mean(w: &[u64], chain: usize) -> f64 {
    let at = CHAINS * PHASE + chain;
    (w[30 + at] * PPU_CYCLES_PER_TB_TICK) as f64 / 5.0 / (1u64 << ITERATIONS_LOG2) as f64
}

/// `mtvscr v0`.
const MTVSCR: u32 = 0x1000_0644;
/// `vaddubm vk, v20, v21`.
const fn vadd(k: u32) -> u32 {
    0x1014_a800 | (k << 21)
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

/// The sibling: an `mtvscr` and seven independent `vaddubm`.
fn sibling() -> Sibling {
    let words = sibling_loop(
        [
            MTVSCR,
            vadd(1),
            vadd(2),
            vadd(3),
            vadd(4),
            vadd(5),
            vadd(6),
            vadd(7),
        ],
        10,
        8,
    );
    Sibling {
        words: Box::leak(Box::new(words)),
        seeds: SIBLING_SEEDS,
    }
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

/// The share of the console's mean run CellGov's cycles may lie on either
/// side of it, as for the sibling of back-to-back `mtvscr`.
const BAND: f64 = 0.25;

#[test]
fn each_load_and_store_chain_beside_a_draining_mtvscr_lies_within_the_band_of_the_mean_run() {
    let w = console_words();
    let sibling = sibling();
    let rows: Vec<(usize, f64, f64)> = chains()
        .iter()
        .enumerate()
        .map(|(c, chain)| {
            let cellgov = replay_beside(chain, 1, &sibling).expect("the chain replays");
            (c, cellgov, console_mean(&w, c))
        })
        .collect();
    assert_eq!(rows.len(), CHAINS);
    for &(c, cellgov, console) in &rows {
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "chain {c}: CellGov {cellgov:.2} cycles an iteration, the console's mean \
             {console:.2}, more than {BAND} of it apart: {rows:?}",
        );
    }
}
