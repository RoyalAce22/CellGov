//! CellGov's PPU timing against the `ppu_issue_barrier_smt` console
//! capture: what a `sync`, `lwsync` or `isync` loop costs beside a sibling
//! thread, and what share of its own rate the sibling keeps beside it.
//!
//! The probe's sibling runs eight independent `addi` in a loop of 32
//! instructions that loads a stop flag and counts its rounds in memory,
//! with no time-base read: the words of the `ppu_issue_sibling` probe's
//! `addi` sibling. Five chains on one line run on the main thread beside
//! it.

use crate::console_ppu_issue_sibling_rate::{cellgov_kept, SIBLINGS};
use crate::console_ppu_issue_timing::{replay_beside, Replay, Seed, Sibling};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_barrier_smt/ps3/cech20-cex-493/cgov_frame.bin";

/// The payload's word 72 when the probe wrote every word.
const DONE: u64 = 0x5050_4253;

/// The thread exit value the sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The chains a phase times.
const CHAINS: usize = 5;

/// The phase that runs the sibling.
const PHASE: usize = 1;

/// The probe's runs of each loop.
const REPS: u64 = 5;

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
    assert_eq!(words[71], REPS, "five runs a loop");
    assert_eq!(words[50 + PHASE], 0, "the sibling was created");
    assert_eq!(words[55 + PHASE], 0, "the sibling joined");
    assert_eq!(words[60 + PHASE], SIBLING_EXIT, "the sibling exited");
    assert_eq!(words[65 + PHASE], 1, "the sibling ran");
    words
}

/// Chain `chain`'s mean run in `phase`, in core cycles an iteration.
fn console_mean(w: &[u64], phase: usize, chain: usize) -> f64 {
    let at = CHAINS * phase + chain;
    (w[30 + at] * PPU_CYCLES_PER_TB_TICK) as f64 / REPS as f64 / (1u64 << ITERATIONS_LOG2) as f64
}

/// The share of its alone rate the sibling keeps beside `chain` on the
/// console.
fn console_kept(w: &[u64], chain: usize) -> f64 {
    let at = CHAINS * PHASE + chain;
    let rate = |rounds: u64, ticks: u64| rounds as f64 / ticks as f64;
    rate(w[20 + at], w[30 + at]) / rate(w[40 + 2 * PHASE], w[41 + 2 * PHASE])
}

/// `std r17, 0(r9)`.
const STD: u32 = 0xfa29_0000;
/// `sync`.
const SYNC: u32 = 0x7c00_04ac;
/// `lwsync`.
const LWSYNC: u32 = 0x7c20_04ac;
/// `isync`.
const ISYNC: u32 = 0x4c00_012c;

/// The line the chains store to, in r9.
const LINE: u64 = 0x4000;

/// `body` closed by its `bdnz`.
fn looped(body: &[u32]) -> &'static [u32] {
    let mut words = body.to_vec();
    words.push(0x4200_0000 | ((4 * body.len() as u32).wrapping_neg() & 0xfffc));
    Box::leak(words.into_boxed_slice())
}

/// Each chain, in the probe's order.
fn chains() -> [Replay; CHAINS] {
    let chain = |k: usize, body: &[u32]| Replay {
        loop_index: k,
        words: looped(body),
        seeds: &[Seed::Gpr(9, LINE)],
    };
    [
        chain(0, &[SYNC]),
        chain(1, &[LWSYNC]),
        chain(2, &[ISYNC]),
        chain(3, &[STD, SYNC]),
        chain(4, &[STD, LWSYNC]),
    ]
}

/// The probe's sibling: the `ppu_issue_sibling` probe's `addi` sibling.
fn sibling() -> &'static Sibling {
    let (name, sibling) = &SIBLINGS[0];
    assert_eq!(*name, "addi");
    sibling
}

/// Chain `chain`'s fastest run with no sibling, in core cycles an
/// iteration.
fn console_alone(w: &[u64], chain: usize) -> f64 {
    (w[2 * chain] * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64
}

/// The share of a chain's fastest run alone the model may lie on either
/// side of it, as for the barrier loops of `ppu_issue_fp_store_drain`.
const ALONE_BAND: f64 = 0.06;

#[test]
fn each_barrier_chain_alone_lies_within_the_band() {
    let w = console_words();
    let rows: Vec<(usize, f64, f64)> = chains()
        .iter()
        .enumerate()
        .map(|(c, chain)| {
            let cellgov = replay_beside(chain, 0, sibling()).expect("the chain replays");
            (c, cellgov, console_alone(&w, c))
        })
        .collect();
    assert_eq!(rows.len(), CHAINS);
    for &(c, cellgov, console) in &rows {
        assert!(
            (cellgov - console).abs() / console <= ALONE_BAND,
            "chain {c}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}: {rows:?}",
        );
    }
}

/// The share of the console's mean run CellGov's cycles may lie on either
/// side of it, as for the chains beside a draining `mtvscr`.
const BAND: f64 = 0.25;

#[test]
fn each_barrier_chain_beside_an_addi_sibling_lies_within_the_band_of_the_mean_run() {
    let w = console_words();
    let rows: Vec<(usize, f64, f64)> = chains()
        .iter()
        .enumerate()
        .map(|(c, chain)| {
            let cellgov = replay_beside(chain, 1, sibling()).expect("the chain replays");
            (c, cellgov, console_mean(&w, PHASE, c))
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

/// The share of its rate the sibling may keep beside a chain, off the
/// console's, on either side, as for the siblings of `ppu_issue_sibling`.
const KEPT_GAP: f64 = 0.20;

/// The chain of an `isync`. Every other chain holds a `sync` or an
/// `lwsync`.
const ISYNC_CHAIN: usize = 2;

/// On the console a `sync` or `lwsync` hold stalls the sibling, which
/// keeps under a tenth of its rate, and an `isync` leaves it most of its
/// rate. The model's sibling keeps most of its rate beside a `sync` or an
/// `lwsync`, and little beside an `isync`.
#[test]
fn the_sibling_s_share_beside_each_barrier_chain_is_a_known_difference() {
    let w = console_words();
    let rows: Vec<(usize, f64, f64)> = chains()
        .iter()
        .enumerate()
        .map(|(c, chain)| (c, cellgov_kept(chain, sibling()), console_kept(&w, c)))
        .collect();
    assert_eq!(rows.len(), CHAINS);
    for &(c, cellgov, console) in &rows {
        let over = if c == ISYNC_CHAIN {
            console - cellgov
        } else {
            cellgov - console
        };
        assert!(
            over > KEPT_GAP,
            "chain {c}: CellGov's sibling keeps {cellgov:.3} of its rate, now within \
             {KEPT_GAP} of the console's {console:.3}; turn the chain into a band test: \
             {rows:?}",
        );
    }
}
