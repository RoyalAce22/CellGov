//! CellGov's PPU timing against the `ppu_issue_lhs_distance` console
//! capture: an `std` and an `ld`, and an `stmw` and an `lmw`, of the same
//! bytes with 0 to 16 independent `li` between them.
//!
//! The probe times eighteen loops with `mftb`, eleven runs each. r6
//! holds the line every store and load addresses.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_lhs_distance/ps3/cech20-cex-493/observation.json";

/// The loops the probe times.
const LOOPS: usize = 18;

/// The payload's word 38 when the probe wrote every word.
const DONE: u64 = 0x5050_4c44;

/// The runs the probe takes of each loop.
const REPS: u64 = 11;

/// log2 of each loop's iterations on the console.
const ITERATIONS_LOG2: u32 = 16;

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

/// `std r9, 0(r6)`.
const STD: u32 = 0xf926_0000;
/// `ld r10, 0(r6)`.
const LD: u32 = 0xe946_0000;
/// `stmw r24, 0(r6)`.
const STMW: u32 = 0xbf06_0000;
/// `lmw r24, 0(r6)`.
const LMW: u32 = 0xbb06_0000;

/// The `li` counts between the store and the load, in the probe's order.
const DISTANCES: [usize; 8] = [0, 1, 2, 4, 6, 8, 12, 16];

/// `li rn, 1` for r16 through r23 in turn, `n` of them.
fn li(n: usize) -> Vec<u32> {
    (0..n)
        .map(|i| 0x3800_0001 | (16 + i as u32 % 8) << 21)
        .collect()
}

/// Each loop's body before its `bdnz`, in the probe's order.
fn bodies() -> Vec<Vec<u32>> {
    let between = |store: u32, n: usize, load: u32| [vec![store], li(n), vec![load]].concat();
    let mut bodies: Vec<Vec<u32>> = DISTANCES.iter().map(|&n| between(STD, n, LD)).collect();
    bodies.extend(DISTANCES.iter().map(|&n| between(STMW, n, LMW)));
    bodies.push([vec![STD], li(16)].concat());
    bodies.push([vec![STMW], li(16)].concat());
    assert_eq!(bodies.len(), LOOPS);
    bodies
}

/// CellGov's cycles an iteration for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let mut words = bodies()[k].clone();
    words.push(0x4200_0000 | ((4 * words.len() as u32).wrapping_neg() & 0xfffc));
    let r = Replay {
        loop_index: k,
        words: Box::leak(words.into_boxed_slice()),
        seeds: &[Seed::Gpr(6, ONE_LINE)],
    };
    replay(&r, 0).expect("the loop replays")
}

/// The share of a loop's fastest console run the model may lie on either
/// side of it.
const BAND: f64 = 0.06;

/// The loops of a store and its 16 `li` with no load after them.
const STORE_ONLY: [usize; 2] = [16, 17];

/// The `stmw` and `lmw` loops with 0 to 4 `li` between.
const LMW_WAITS: std::ops::Range<usize> = 8..12;
/// The `stmw` and `lmw` loops with 6 to 16 `li` between.
const LMW_DOES_NOT_WAIT: std::ops::Range<usize> = 12..16;

#[test]
fn a_store_and_its_li_with_no_load_lie_within_the_band() {
    let w = words_at(CONSOLE);
    for k in STORE_ONLY {
        let (cellgov, console) = (cellgov_cycles(k), console_cycles(&w, k).0);
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}",
        );
    }
}

#[test]
fn an_lmw_six_li_or_more_after_an_stmw_does_not_wait_for_it_on_the_console() {
    // With 0 to 4 `li` between, the `lmw` waits for the `stmw`'s entries
    // and each loop takes over 95 cycles an iteration. With 6 to 16 it
    // costs about what an `lmw` alone does, and no loop takes 70.
    let w = words_at(CONSOLE);
    for k in LMW_WAITS {
        let console = console_cycles(&w, k).0;
        assert!(
            console > 95.0,
            "loop {k}: the console's fastest {console:.2}"
        );
    }
    for k in LMW_DOES_NOT_WAIT {
        let console = console_cycles(&w, k).0;
        assert!(
            console < 70.0,
            "loop {k}: the console's fastest {console:.2}"
        );
    }
}

/// The model gives every load after a store of its bytes the same wait,
/// whatever lies between them. Against the console's fastest run:
///
/// - each `ld` loop lies under it by more than the band;
/// - the `lmw` loops with 0 to 4 `li` lie on either side of it;
/// - the `lmw` loops with 6 or more `li` lie about twice over it.
#[test]
fn every_load_after_a_store_of_its_bytes_is_a_known_difference() {
    let w = words_at(CONSOLE);
    for k in 0..LOOPS - STORE_ONLY.len() {
        let (cellgov, console) = (cellgov_cycles(k), console_cycles(&w, k).0);
        assert!(
            (cellgov - console).abs() / console > BAND,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration now lies within the band \
             of the console's {console:.2}",
        );
    }
}
