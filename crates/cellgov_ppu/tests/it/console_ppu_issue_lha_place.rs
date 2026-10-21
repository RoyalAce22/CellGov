//! CellGov's PPU timing against the `ppu_issue_lha_place` console
//! capture: a sibling of microcoded `lha` whose loop head sits at each
//! 16-byte offset of a 128-byte line, and once with its loads on the main
//! thread's `ld` line, beside dependent `add` and an L1 `ld` chase.
//!
//! The sibling's words are the `ppu_issue_sibling` probe's `lha` loop
//! (base r10, stop flag r8, round count r7); the replay runs them once,
//! at its own address.

use crate::console_ppu_issue_sibling_rate::{CHAINS_TIMED, SIBLINGS};
use crate::console_ppu_issue_timing::replay_beside;
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_lha_place/ps3/cech20-cex-493/cgov_frame.bin";

/// The payload's word 152 when the probe wrote every word.
const DONE: u64 = 0x5050_534F;

/// The thread exit value each sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The probe's repetitions of each loop.
const REPS: u64 = 5;

/// Each timed loop's iterations on the console.
const ITERATIONS: u64 = 1 << 16;

/// Instructions a timed loop's iteration runs before its `bdnz`.
const BODY: f64 = 8.0;

/// The sibling phases: eight loop offsets, then the shared `ld` line.
const PHASES: usize = 9;

/// The chains the probe times, as indexes into [`CHAINS_TIMED`]: `add`
/// and `ld`.
const CHAINS: [usize; 2] = [0, 4];

/// A cell counts as one where the sibling held its thread when the
/// sibling keeps this share of its rate and the cell's runs are narrow.
const HELD: f64 = 0.8;
const NARROW: f64 = 0.7;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let words: Vec<u64> = frame[8..]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words.len(), 160);
    assert_eq!(words[152], DONE, "the probe wrote every word");
    assert_eq!(words[151], REPS, "five runs a loop");
    for p in 1..=PHASES {
        assert_eq!(words[100 + p], 0, "phase {p}'s sibling was created");
        assert_eq!(words[110 + p], 0, "phase {p}'s sibling joined");
        assert_eq!(words[120 + p], SIBLING_EXIT, "phase {p}'s sibling exited");
        assert_eq!(words[130 + p], 1, "phase {p}'s sibling ran");
        assert_eq!(words[140 + p], 0, "phase {p}'s sleep returned");
    }
    words
}

/// One cell of the capture.
struct Cell {
    phase: usize,
    chain: usize,
    /// The cycles each instruction of the chain waits past its alone time.
    excess: f64,
    /// The chain's mean cycles an iteration beside the sibling.
    mean: f64,
}

/// The cells where the sibling held its thread.
fn held_cells(w: &[u64]) -> Vec<Cell> {
    let mean = |l: usize| (w[60 + l] * PPU_CYCLES_PER_TB_TICK) as f64 / (REPS * ITERATIONS) as f64;
    let mut cells = Vec::new();
    for phase in 1..=PHASES {
        let alone_rate = w[80 + 2 * phase] as f64 / w[81 + 2 * phase] as f64;
        for chain in 0..CHAINS.len() {
            let l = 2 * phase + chain;
            let kept = (w[40 + l] as f64 / w[60 + l] as f64) / alone_rate;
            let (fastest, slowest) = (w[2 * l] as f64, w[2 * l + 1] as f64);
            if kept >= HELD && fastest >= NARROW * slowest {
                cells.push(Cell {
                    phase,
                    chain,
                    excess: (mean(l) - mean(chain)) / BODY,
                    mean: mean(l),
                });
            }
        }
    }
    cells
}

#[test]
fn where_the_sibling_holds_its_thread_its_loop_offset_and_load_line_set_no_level() {
    // Every held cell waits 13.0 to 14.8 cycles an instruction: the `ld`
    // chain at seven of the eight loop offsets (all but +0x30), the `add`
    // chain at +0x40 to +0x60, and both with the loads on the main
    // thread's line. The other `add` cells ran partly with the sibling off
    // its thread.
    let w = console_words();
    let cells = held_cells(&w);
    let held: Vec<(usize, usize)> = cells.iter().map(|c| (c.phase, c.chain)).collect();
    let mut named: Vec<(usize, usize)> = [1, 2, 3, 5, 6, 7, 8, 9].map(|p| (p, 1)).to_vec();
    named.extend([5, 6, 7, 9].map(|p| (p, 0)));
    named.sort_unstable();
    assert_eq!(held, named, "held (phase, chain) cells");
    let (lo, hi) = cells.iter().fold((f64::MAX, f64::MIN), |(lo, hi), c| {
        (lo.min(c.excess), hi.max(c.excess))
    });
    assert!(hi - lo < 2.5, "held excess {lo:.2} to {hi:.2}");
    assert!(lo > 12.5 && hi < 15.0, "held excess {lo:.2} to {hi:.2}");
}

#[test]
fn beside_a_held_microcoded_sibling_the_model_waits_short_of_this_capture() {
    // An accepted gap: the `lha` level moves with the probe, 7 to 10 in
    // `ppu_issue_units` and `ppu_issue_layout`, 12.5 to 14.4 in
    // `ppu_issue_shared`, 13 to 15 here. The model's microcode stall of
    // 10 sits at the top of the lower range.
    let w = console_words();
    let (_, lha) = &SIBLINGS[5];
    for cell in held_cells(&w) {
        let (name, replay) = &CHAINS_TIMED[CHAINS[cell.chain]];
        let cellgov = replay_beside(replay, 1, lha).expect("the chain replays");
        assert!(
            cellgov < cell.mean && cellgov > 0.6 * cell.mean,
            "{name} beside lha phase {}: CellGov {cellgov:.1} cycles an iteration, the \
             console's mean {:.1}",
            cell.phase,
            cell.mean,
        );
    }
}
