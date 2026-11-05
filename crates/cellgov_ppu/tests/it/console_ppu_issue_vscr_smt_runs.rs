//! CellGov's PPU timing against the `ppu_issue_vscr_smt_runs` console
//! capture: what a chain of independent `vaddubm`, or of `li`, costs an
//! iteration beside a sibling thread whose vector work waits behind its
//! own `mtvscr`.
//!
//! The probe records every run and the sibling's rounds over it, so the
//! tests can omit a run the sibling did not share.

use crate::console_ppu_issue_sibling_rate::{COUNT, STOP};
use crate::console_ppu_issue_timing::{replay_beside, Replay, Seed, Sibling};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_vscr_smt_runs/ps3/cech20-cex-493/observation.json";

/// The payload's word 566 when the probe wrote every word.
const DONE: u64 = 0x5050_5652;

/// The number of `vaddubm` after the `mtvscr` in the sibling of phases 1 and 2.
const SIBLINGS: [u32; 2] = [1, 7];

/// The thread exit value each sibling passes.
pub(crate) const SIBLING_EXIT: u64 = 0x5157;

/// The phases: none, then the two siblings.
pub(crate) const PHASES: usize = 3;

/// The chains a phase times.
pub(crate) const CHAINS: usize = 10;

/// The runs of each chain.
const REPS: usize = 9;

/// The word each phase's record starts at.
pub(crate) const META: usize = 540;

/// log2 of each chain's iterations on the console.
const ITERATIONS_LOG2: u32 = 16;

/// The payload of the capture at `path` as big-endian words.
///
/// Word 566 holds `done` when the probe wrote every word.
pub(crate) fn words_at(path: &str, done: u64) -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    let words: Vec<u64> = observation.memory_regions[0]
        .data
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words.len(), 567);
    assert_eq!(words[566], done, "the probe wrote every word");
    assert_eq!(
        (words[564], words[565]),
        (REPS as u64, u64::from(ITERATIONS_LOG2))
    );
    words
}

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    words_at(CONSOLE, DONE)
}

/// One run: its cycles an iteration, and the sibling's rounds a time-base
/// tick over it.
fn run(w: &[u64], phase: usize, chain: usize, r: usize) -> (f64, f64) {
    let at = 2 * (REPS * (CHAINS * phase + chain) + r);
    let ticks = w[at];
    let cycles = (ticks * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64;
    (cycles, w[at + 1] as f64 / ticks as f64)
}

/// The sibling's rounds a time-base tick while the main thread slept.
fn alone_rate(w: &[u64], phase: usize) -> f64 {
    let m = META + 8 * phase;
    w[m] as f64 / w[m + 1] as f64
}

/// The runs of the chain beside the sibling, in cycles an iteration,
/// sorted: those in which the sibling kept at least half its rate alone.
pub(crate) fn shared_runs(w: &[u64], phase: usize, chain: usize) -> Vec<f64> {
    let floor = alone_rate(w, phase) / 2.0;
    let mut runs: Vec<f64> = (0..REPS)
        .map(|r| run(w, phase, chain, r))
        .filter(|&(_, rate)| rate >= floor)
        .map(|(cycles, _)| cycles)
        .collect();
    runs.sort_by(f64::total_cmp);
    runs
}

/// The median of sorted `xs`.
pub(crate) fn median(xs: &[f64]) -> f64 {
    let n = xs.len();
    if n % 2 == 1 {
        xs[n / 2]
    } else {
        (xs[n / 2 - 1] + xs[n / 2]) / 2.0
    }
}

/// `vaddubm vk, v20, v21`.
const fn vadd(k: u32) -> u32 {
    0x1014_a800 | (k << 21)
}

/// `mtvscr v0`.
const MTVSCR: u32 = 0x1000_0644;

/// `li r17, 1` to `li r20, 1`.
const LI: [u32; 4] = [0x3a20_0001, 0x3a40_0001, 0x3a60_0001, 0x3a80_0001];

/// `n` independent `vaddubm`, cycling v1 to v`targets`.
fn independent(n: u32, targets: u32) -> Vec<u32> {
    (0..n).map(|i| vadd(1 + i % targets)).collect()
}

/// `n` `li`, cycling r17 to r20.
fn li(n: usize) -> Vec<u32> {
    LI.iter().copied().cycle().take(n).collect()
}

/// The compiled sibling's loop: `li r14,4; mtctr r14`, then `body` to a
/// `bdnz`. The loop then loads the stop flag at r10, increments the round
/// count at r8 and branches back.
fn sibling(body: &[u32]) -> Sibling {
    let mut w = vec![0x39c0_0004, 0x7dc9_03a6];
    w.extend_from_slice(body);
    w.push(0x4200_0000 | ((4 * body.len() as u32).wrapping_neg() & 0xfffc));
    w.extend_from_slice(&[
        0x8000_0000 | (10 << 16),
        0x2c00_0000,
        0x4082_0014,
        0xe920_0000 | (8 << 16),
        0x3929_0001,
        0xf920_0000 | (8 << 16),
    ]);
    w.push(0x4800_0000 | ((4 * w.len() as u32).wrapping_neg() & 0x03ff_fffc));
    Sibling {
        words: Box::leak(w.into_boxed_slice()),
        seeds: &[Seed::Gpr(10, STOP), Seed::Gpr(8, COUNT)],
    }
}

/// A sibling of an `mtvscr` and `n` independent `vaddubm`.
fn sibling_of(n: u32) -> Sibling {
    sibling(&[&[MTVSCR][..], &independent(n, 7)].concat())
}

/// `body` closed by its `bdnz`.
fn looped(body: &[u32]) -> &'static [u32] {
    let mut words = body.to_vec();
    words.push(0x4200_0000 | ((4 * body.len() as u32).wrapping_neg() & 0xfffc));
    Box::leak(words.into_boxed_slice())
}

/// Chain `chain`'s body before its `bdnz`, in the probe's order.
fn body(chain: usize) -> Vec<u32> {
    match chain {
        0 => independent(1, 8),
        1 => independent(2, 8),
        2 => independent(3, 8),
        3 => independent(4, 8),
        4 => independent(6, 8),
        5 => independent(8, 8),
        6 => independent(12, 8),
        7 => independent(16, 8),
        8 => [independent(2, 8), li(8)].concat(),
        _ => li(8),
    }
}

/// CellGov's cycles an iteration of chain `chain` beside a sibling of an
/// `mtvscr` and `n` independent `vaddubm`.
pub(crate) fn cellgov_beside_sibling(n: u32, chain: usize) -> f64 {
    let r = Replay {
        loop_index: chain,
        words: looped(&body(chain)),
        seeds: &[],
    };
    replay_beside(&r, 1, &sibling_of(n)).expect("the chain replays")
}

/// CellGov's cycles an iteration of chain `chain` beside the sibling of
/// `phase`.
fn cellgov_beside(phase: usize, chain: usize) -> f64 {
    cellgov_beside_sibling(SIBLINGS[phase - 1], chain)
}

/// The fewest runs of a chain the sibling must share for its median to
/// count.
const SHARED_FLOOR: usize = 5;

#[test]
fn every_sibling_ran_and_shared_enough_runs_of_each_chain() {
    let w = console_words();
    for p in 1..PHASES {
        let m = META + 8 * p;
        assert_eq!(
            (w[m + 2], w[m + 3], w[m + 4], w[m + 5], w[m + 6]),
            (0, 0, SIBLING_EXIT, 1, 0),
            "phase {p}: the sibling's create rc, join rc, exit value, start and sleep rc",
        );
        for c in 0..CHAINS {
            let shared = shared_runs(&w, p, c).len();
            assert!(
                shared >= SHARED_FLOOR,
                "phase {p} chain {c}: {shared} shared runs"
            );
        }
    }
}

#[test]
fn beside_a_fenced_sibling_eight_li_cost_the_console_about_what_eight_vaddubm_do() {
    // Alone, the 8 `li` loop and the 8 `vaddubm` loop each take 12.0
    // cycles an iteration.
    let w = console_words();
    for p in 1..PHASES {
        let (li, vadd) = (
            median(&shared_runs(&w, p, 9)),
            median(&shared_runs(&w, p, 5)),
        );
        assert!(
            (li - vadd).abs() / vadd < 0.2,
            "phase {p}: 8 li {li:.2}, 8 vaddubm {vadd:.2} cycles an iteration",
        );
    }
}

/// The share of the console's median shared run a held cell may lie on
/// either side of it, as in `console_ppu_issue_vscr_smt`.
const BAND: f64 = 0.25;

/// The cells, as (phase, chain), the model times more than [`BAND`] under
/// the console's median shared run. Each is beside the sibling of an
/// `mtvscr` and one `vaddubm`, whose runs spread widely:
///
/// - the 4 `vaddubm` chain: the runs span 44.51 to 88.97 cycles an
///   iteration;
/// - the chains of 2 `vaddubm` and 8 `li`, and of 8 `li`: the console
///   charges each `li` about what it charges a `vaddubm`, and the model
///   charges each `li` less.
///
/// When a difference closes, the test fails. Then move the cell into the
/// held set.
const DIFFERENCES: [(usize, usize); 3] = [(1, 3), (1, 8), (1, 9)];

/// Each cell beside a sibling, with CellGov's cycles and the console's
/// median shared run.
fn cells() -> Vec<(usize, usize, f64, f64)> {
    let w = console_words();
    (1..PHASES)
        .flat_map(|p| (0..CHAINS).map(move |c| (p, c)))
        .map(|(p, c)| (p, c, cellgov_beside(p, c), median(&shared_runs(&w, p, c))))
        .collect()
}

#[test]
fn each_held_chain_beside_a_fenced_sibling_lies_within_the_band_of_the_median_shared_run() {
    let rows = cells();
    let held: Vec<_> = rows
        .iter()
        .filter(|(p, c, ..)| !DIFFERENCES.contains(&(*p, *c)))
        .collect();
    assert_eq!(held.len(), (PHASES - 1) * CHAINS - DIFFERENCES.len());
    for &&(p, c, cellgov, console) in &held {
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "phase {p} chain {c}: CellGov {cellgov:.2} cycles an iteration, the console's \
             median {console:.2}, more than {BAND} of it apart: {rows:?}",
        );
    }
}

#[test]
fn each_other_chain_beside_a_fenced_sibling_is_a_known_difference() {
    let rows = cells();
    for (p, c) in DIFFERENCES {
        let &(_, _, cellgov, console) = rows
            .iter()
            .find(|row| (row.0, row.1) == (p, c))
            .expect("the cell");
        assert!(
            cellgov < (1.0 - BAND) * console,
            "phase {p} chain {c}: CellGov {cellgov:.2} cycles an iteration now lies near the \
             console's median {console:.2}; move the cell into the held set",
        );
    }
}
