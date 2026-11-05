//! CellGov's PPU timing against the `ppu_issue_multiple_pair_addi`
//! console capture: an 8-word `stmw` and an `lmw` of the same bytes, with
//! independent `addi` before, between or after them.
//!
//! The probe times twelve loops with `mftb`, eleven runs each. r8 holds
//! the line the pair stores to and loads from.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_multiple_pair_addi/ps3/cech20-cex-493/observation.json";

/// The loops the probe times.
const LOOPS: usize = 12;

/// The payload's word 26 when the probe wrote every word.
const DONE: u64 = 0x5050_504d;

/// The runs the probe takes of each loop.
const REPS: u64 = 11;

/// log2 of each loop's iterations on the console.
const ITERATIONS_LOG2: u32 = 17;

/// The payload of the capture at `path` as big-endian words.
fn words_at(path: &str) -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 8 * (2 * LOOPS + 3));
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[2 * LOOPS + 2], DONE, "the probe wrote every word");
    assert_eq!(
        (words[2 * LOOPS], words[2 * LOOPS + 1]),
        (REPS, u64::from(ITERATIONS_LOG2))
    );
    words
}

/// Loop `k`'s fastest and slowest runs, in core cycles an iteration.
fn console_cycles(words: &[u64], k: usize) -> (f64, f64) {
    let per = |t: u64| (t * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64;
    (per(words[2 * k]), per(words[2 * k + 1]))
}

/// `stmw r24, 0(r8)`.
const STMW: u32 = 0xbf08_0000;
/// `lmw r24, 0(r8)`.
const LMW: u32 = 0xbb08_0000;

/// `addi rn, rn, 1` for r16 through r23 in turn, `n` of them.
fn addi(n: usize) -> Vec<u32> {
    (0..n)
        .map(|i| 0x3800_0001 | ((16 + i as u32 % 8) * 0x21) << 16)
        .collect()
}

/// Each loop's body before its `bdnz`, in the probe's order.
fn bodies() -> [Vec<u32>; LOOPS] {
    let pair_then = |n: usize| [vec![STMW, LMW], addi(n)].concat();
    [
        pair_then(0),
        pair_then(1),
        pair_then(2),
        pair_then(4),
        pair_then(8),
        pair_then(16),
        [addi(8), vec![STMW, LMW]].concat(),
        [vec![STMW], addi(8), vec![LMW]].concat(),
        vec![STMW],
        vec![LMW],
        [vec![STMW], addi(8)].concat(),
        pair_then(0),
    ]
}

/// CellGov's cycles an iteration for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let mut words = bodies()[k].clone();
    words.push(0x4200_0000 | ((4 * words.len() as u32).wrapping_neg() & 0xfffc));
    let r = Replay {
        loop_index: k,
        words: Box::leak(words.into_boxed_slice()),
        seeds: &[Seed::Gpr(8, ONE_LINE)],
    };
    replay(&r, 0).expect("the loop replays")
}

/// The share of a loop's fastest console run the model may lie on either
/// side of it.
const BAND: f64 = 0.06;

/// The loops of an `stmw` alone and of an `stmw` and 8 `addi`.
const NO_LOAD: [usize; 2] = [8, 10];

#[test]
fn an_stmw_with_no_lmw_lies_within_the_band() {
    let w = words_at(CONSOLE);
    for k in NO_LOAD {
        let (cellgov, console) = (cellgov_cycles(k), console_cycles(&w, k).0);
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}",
        );
    }
}

#[test]
fn addi_after_the_lmw_cost_what_the_same_addi_before_the_stmw_do() {
    // The loop runs the same instructions in the same order either way,
    // so the console times both within 1%. The first and the last loop
    // of the probe run the pair alone and agree to the same 1%.
    let w = words_at(CONSOLE);
    for (a, b) in [(4, 6), (0, 11)] {
        let (x, y) = (console_cycles(&w, a).0, console_cycles(&w, b).0);
        assert!(
            (x - y).abs() / x < 0.01,
            "loops {a} and {b}: {x:.2} and {y:.2}"
        );
    }
}

/// The pair loops the model times outside the band: every loop of the
/// pair but those with 4 and 16 `addi` after it, and the `lmw` alone.
///
/// The console's cost of the pair, in cycles an iteration, moves with the
/// `addi` between the `lmw` and the next `stmw`, and not in one direction:
///
/// - 0 or 1 `addi`: 137.5;
/// - 2 `addi`: 101;
/// - 4 `addi`: 121;
/// - 8 `addi`: 107;
/// - 16 `addi`: 115.
///
/// With 8 `addi` between the `stmw` and the `lmw`, the `lmw` does not wait
/// for the store at all. The model gives the `lmw` the same wait in every
/// loop.
const KNOWN: [usize; 8] = [0, 1, 2, 4, 6, 7, 9, 11];

/// The pair loops with 4 and 16 `addi` after the `lmw`, whose console
/// cost lies within the band of the model's single wait.
const PAIR_IN_BAND: [usize; 2] = [3, 5];

#[test]
fn the_pair_with_4_or_16_addi_after_it_lies_within_the_band() {
    let w = words_at(CONSOLE);
    for k in PAIR_IN_BAND {
        let (cellgov, console) = (cellgov_cycles(k), console_cycles(&w, k).0);
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}",
        );
    }
}

#[test]
fn the_pair_with_addi_around_it_is_a_known_difference() {
    let w = words_at(CONSOLE);
    for k in KNOWN {
        let (cellgov, console) = (cellgov_cycles(k), console_cycles(&w, k).0);
        assert!(
            (cellgov - console).abs() / console > BAND,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration now lies within the band \
             of the console's {console:.2}",
        );
    }
}
