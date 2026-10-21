//! CellGov's PPU timing against the `ppu_issue_multiply` console capture.
//!
//! The probe times seven chains on the main thread, alone and beside a
//! sibling thread of dependent `divd`, dependent `fdiv` or dependent
//! `mulld`. The replay runs each chain beside one PPU unit that runs the
//! sibling's loop.

use crate::console_ppu_issue_shared::{eight, SIBLINGS};
use crate::console_ppu_issue_timing::{replay_beside, Replay, Seed, Sibling};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_multiply/ps3/cech20-cex-493/cgov_frame.bin";

/// Each timed loop's iterations on the console, from the probe's header.
const ITERATIONS: u64 = 1 << 16;

/// The probe's repetitions of each loop.
const REPS: u64 = 5;

/// The payload's word 138 when the probe wrote every word.
const DONE: u64 = 0x5050_4d55;

/// The thread exit value each sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The chains a phase times.
const CHAINS: usize = 7;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let words: Vec<u64> = frame[8..]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words.len(), 144);
    assert_eq!(words[138], DONE, "the probe wrote every word");
    assert_eq!(words[137], REPS, "five runs a loop");
    for p in 1..4 {
        assert_eq!(words[120 + p], 0, "phase {p}'s sibling was created");
        assert_eq!(words[124 + p], 0, "phase {p}'s sibling joined");
        assert_eq!(words[128 + p], SIBLING_EXIT, "phase {p}'s sibling exited");
        assert_eq!(words[132 + p], 1, "phase {p}'s sibling ran");
    }
    words
}

/// Loop `CHAINS * phase + chain`'s cycles an iteration on the console:
/// the fastest run, the slowest, and the mean over its runs.
fn console_cycles(words: &[u64], phase: usize, chain: usize) -> (f64, f64, f64) {
    let at = CHAINS * phase + chain;
    let per = |tb: u64, iterations: u64| (tb * PPU_CYCLES_PER_TB_TICK) as f64 / iterations as f64;
    (
        per(words[2 * at], ITERATIONS),
        per(words[2 * at + 1], ITERATIONS),
        per(words[84 + at], REPS * ITERATIONS),
    )
}

const BDNZ_16: u32 = 0x4200_ffc0;

const CHAIN_MULLI: [u32; 9] = eight(0x1d29_0001);
const CHAIN_MULLI_IND: [u32; 9] = [
    0x1e31_0001,
    0x1e52_0001,
    0x1e73_0001,
    0x1e94_0001,
    0x1e31_0001,
    0x1e52_0001,
    0x1e73_0001,
    0x1e94_0001,
    0x4200_ffe0,
];
const CHAIN_MULLW: [u32; 9] = eight(0x7d29_79d6);
const CHAIN_MULLD: [u32; 9] = eight(0x7d29_79d2);
const CHAIN_MULHW: [u32; 9] = eight(0x7d29_7896);
const MULLI: u32 = 0x1d29_0001;
const ADD_R16: u32 = 0x7e10_7a14;
const CHAIN_MULLI_ADD: [u32; 17] = [
    MULLI, ADD_R16, MULLI, ADD_R16, MULLI, ADD_R16, MULLI, ADD_R16, MULLI, ADD_R16, MULLI, ADD_R16,
    MULLI, ADD_R16, MULLI, ADD_R16, BDNZ_16,
];
const CHAIN_ADD: [u32; 9] = eight(0x7d29_7a14);

/// The chains' seeds: the chained register at 7, r15 at 1.
const SEEDS: &[Seed] = &[Seed::Gpr(9, 7), Seed::Gpr(15, 1)];
const IND_SEEDS: &[Seed] = &[
    Seed::Gpr(17, 7),
    Seed::Gpr(18, 7),
    Seed::Gpr(19, 7),
    Seed::Gpr(20, 7),
];

const fn chain(loop_index: usize, words: &'static [u32], seeds: &'static [Seed]) -> Replay {
    Replay {
        loop_index,
        words,
        seeds,
    }
}

/// The probe's chains, in its order, and their names.
const CHAINS_TIMED: [(&str, Replay); CHAINS] = [
    ("mulli", chain(0, &CHAIN_MULLI, SEEDS)),
    ("mulli independent", chain(1, &CHAIN_MULLI_IND, IND_SEEDS)),
    ("mullw", chain(2, &CHAIN_MULLW, SEEDS)),
    ("mulld", chain(3, &CHAIN_MULLD, SEEDS)),
    ("mulhw", chain(4, &CHAIN_MULHW, SEEDS)),
    ("mulli and add", chain(5, &CHAIN_MULLI_ADD, SEEDS)),
    ("add", chain(6, &CHAIN_ADD, SEEDS)),
];

/// The probe's siblings, phases 1 to 3, from `ppu_issue_shared`'s.
fn siblings() -> [(&'static str, &'static Sibling); 3] {
    let named = |name: &str| {
        &SIBLINGS
            .iter()
            .find(|(n, _)| *n == name)
            .expect("a ppu_issue_shared sibling")
            .1
    };
    [
        ("divd", named("divd")),
        ("fdiv", named("fdiv")),
        ("mulld", named("mulld")),
    ]
}

/// The largest share of a cell's mean console time the model may sit
/// from it, for a cell whose console runs spread over less.
const TOLERANCE: f64 = 0.20;

/// The cells the model times outside both the console's spread and
/// [`TOLERANCE`], as (sibling, chain). None is a multiply chain:
///
/// - The dependent `add` chain beside each sibling. The console's cost of
///   it moves with the probe: beside `fdiv` each `add` waits 5 to 7
///   cycles here and 9 to 14 in `ppu_issue_shared`, against the model's
///   13.
/// - The `mulli` chain with an independent `add` after each, beside
///   `divd`: the console gives each `add` about what it gives a `mulli`,
///   and the model 13.
const RESIDUALS: [(&str, &str); 4] = [
    ("divd", "add"),
    ("fdiv", "add"),
    ("mulld", "add"),
    ("divd", "mulli and add"),
];

/// How far from the console's mean a residual may sit.
const RESIDUAL_LIMIT: f64 = 1.0;

/// Each chain's CellGov cycles an iteration beside each sibling, with
/// the console's fastest, slowest and mean.
fn rows() -> Vec<(&'static str, &'static str, f64, f64, f64, f64)> {
    let w = console_words();
    let mut rows = Vec::new();
    for (p, (sibling, s)) in siblings().into_iter().enumerate() {
        for (c, (chain, r)) in CHAINS_TIMED.iter().enumerate() {
            let cellgov = replay_beside(r, 1, s).expect("the chain replays");
            let (fastest, slowest, mean) = console_cycles(&w, p + 1, c);
            rows.push((sibling, *chain, cellgov, fastest, slowest, mean));
        }
    }
    rows
}

#[test]
fn the_issue_model_times_each_multiply_chain_beside_each_sibling_near_the_console() {
    let rows = rows();
    assert_eq!(rows.len(), 3 * CHAINS);
    let mut residuals = 0;
    for &(sibling, chain, cellgov, fastest, slowest, mean) in &rows {
        let off = (cellgov - mean).abs() / mean;
        let near = (fastest..=slowest).contains(&cellgov) || off <= TOLERANCE;
        if RESIDUALS.contains(&(sibling, chain)) {
            residuals += 1;
            assert!(
                !near,
                "{chain} beside {sibling} now lands near the console; drop it from the \
                 residuals: {rows:?}",
            );
            assert!(
                off <= RESIDUAL_LIMIT,
                "{chain} beside {sibling}: CellGov {cellgov:.2} cycles an iteration, the \
                 console's mean {mean:.2}: {rows:?}",
            );
        } else {
            assert!(
                near,
                "{chain} beside {sibling}: CellGov {cellgov:.2} cycles an iteration, the \
                 console {fastest:.2} to {slowest:.2}, mean {mean:.2}: {rows:?}",
            );
        }
    }
    assert_eq!(residuals, RESIDUALS.len(), "every residual was timed");
}

#[test]
fn beside_a_divide_sibling_a_longer_multiply_loses_more() {
    // The console's order beside divd and fdiv: mulli, then mullw, then
    // mulld, each chain's mean above the one before.
    let rows = rows();
    for sibling in ["divd", "fdiv"] {
        let cell = |chain: &str| {
            *rows
                .iter()
                .find(|r| r.0 == sibling && r.1 == chain)
                .expect("a timed cell")
        };
        let order = ["mulli", "mullw", "mulld"].map(cell);
        for pair in order.windows(2) {
            assert!(pair[0].5 < pair[1].5, "console beside {sibling}: {order:?}");
            assert!(pair[0].2 < pair[1].2, "CellGov beside {sibling}: {order:?}");
        }
    }
}
