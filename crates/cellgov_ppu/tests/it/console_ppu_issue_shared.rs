//! CellGov's PPU timing against the `ppu_issue_shared` console capture.
//!
//! The probe times five chains on the main thread, alone and beside a
//! sibling thread that runs one instruction class: independent `addi`,
//! dependent or independent `divd`, `divw`, `mulld`, `fdiv`, `fsqrt`, or
//! microcoded `lha`. The replay runs each chain beside one PPU unit that
//! runs the sibling's loop.

use crate::console_ppu_issue_timing::{replay_beside, Replay, Seed, Sibling, BDNZ_8};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_shared/ps3/cech20-cex-493/cgov_frame.bin";

/// Each timed loop's iterations on the console, from the probe's header.
const ITERATIONS: u64 = 1 << 16;

/// The probe's repetitions of each loop.
const REPS: u64 = 5;

/// The payload's word 236 when the probe wrote every word.
const DONE: u64 = 0x5050_5348;

/// The thread exit value each sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let words: Vec<u64> = frame[8..]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words.len(), 240);
    assert_eq!(words[236], DONE, "the probe wrote every word");
    assert_eq!(words[235], REPS, "five runs a loop");
    for p in 1..9 {
        assert_eq!(words[198 + p], 0, "phase {p}'s sibling was created");
        assert_eq!(words[207 + p], 0, "phase {p}'s sibling joined");
        assert_eq!(words[216 + p], SIBLING_EXIT, "phase {p}'s sibling exited");
        assert_eq!(words[225 + p], 1, "phase {p}'s sibling ran");
        for c in 0..5 {
            assert!(
                words[90 + 5 * p + c] > 0,
                "phase {p}'s sibling ran rounds beside chain {c}"
            );
        }
    }
    words
}

/// Loop `5 * phase + chain`'s cycles an iteration on the console: the
/// fastest run, the slowest, and the mean over its runs.
fn console_cycles(words: &[u64], phase: usize, chain: usize) -> (f64, f64, f64) {
    let at = 5 * phase + chain;
    let per = |tb: u64, iterations: u64| (tb * PPU_CYCLES_PER_TB_TICK) as f64 / iterations as f64;
    (
        per(words[2 * at], ITERATIONS),
        per(words[2 * at + 1], ITERATIONS),
        per(words[135 + at], REPS * ITERATIONS),
    )
}

/// Eight copies of `word`, then the loop's `bdnz`.
pub(crate) const fn eight(word: u32) -> [u32; 9] {
    [word, word, word, word, word, word, word, word, BDNZ_8]
}

/// `addi r17..r20,1` in turn, as the probe lays them out.
const ADDI_4: [u32; 4] = [0x3a31_0001, 0x3a52_0001, 0x3a73_0001, 0x3a94_0001];

const CHAIN_ADD: [u32; 9] = eight(0x7d29_7a14);
const CHAIN_MULLI: [u32; 9] = eight(0x1d29_0001);
const CHAIN_ADDI: [u32; 17] = [
    ADDI_4[0],
    ADDI_4[1],
    ADDI_4[2],
    ADDI_4[3],
    ADDI_4[0],
    ADDI_4[1],
    ADDI_4[2],
    ADDI_4[3],
    ADDI_4[0],
    ADDI_4[1],
    ADDI_4[2],
    ADDI_4[3],
    ADDI_4[0],
    ADDI_4[1],
    ADDI_4[2],
    ADDI_4[3],
    0x4200_ffc0,
];
const CHAIN_FADD: [u32; 9] = eight(0xfc00_602a);
const CHAIN_LD: [u32; 9] = eight(0xe929_0000);

/// 1.0 and 1.5 as doubles.
const ONE: u64 = 0x3ff0_0000_0000_0000;
const ONE_HALF: u64 = 0x3ff8_0000_0000_0000;

/// A line whose first doubleword points at itself.
const SELF_LINE: u64 = 0x4000;

/// The probe's chains, in its order.
const CHAINS: [Replay; 5] = [
    Replay {
        loop_index: 0,
        words: &CHAIN_ADD,
        seeds: &[Seed::Gpr(9, 1), Seed::Gpr(15, 3)],
    },
    Replay {
        loop_index: 1,
        words: &CHAIN_MULLI,
        seeds: &[Seed::Gpr(9, 7)],
    },
    Replay {
        loop_index: 2,
        words: &CHAIN_ADDI,
        seeds: &[],
    },
    Replay {
        loop_index: 3,
        words: &CHAIN_FADD,
        seeds: &[Seed::Fpr(0, ONE), Seed::Fpr(12, 0)],
    },
    Replay {
        loop_index: 4,
        words: &CHAIN_LD,
        seeds: &[Seed::Ring(9, SELF_LINE, 128, false)],
    },
];

const NAMES: [&str; 5] = ["add", "mulli", "addi", "fadd", "ld"];

const SIB_ADDI: [u32; 9] = [
    ADDI_4[0], ADDI_4[1], ADDI_4[2], ADDI_4[3], ADDI_4[0], ADDI_4[1], ADDI_4[2], ADDI_4[3], BDNZ_8,
];
const SIB_DIVD: [u32; 9] = eight(0x7def_83d2);
const SIB_DIVD_IND: [u32; 9] = [
    0x7e2f_83d2,
    0x7e4f_83d2,
    0x7e6f_83d2,
    0x7e8f_83d2,
    0x7e2f_83d2,
    0x7e4f_83d2,
    0x7e6f_83d2,
    0x7e8f_83d2,
    BDNZ_8,
];
const SIB_DIVW: [u32; 9] = eight(0x7def_83d6);
const SIB_MULLD: [u32; 9] = eight(0x7def_81d2);
const SIB_FDIV: [u32; 9] = eight(0xfc00_6024);
const SIB_FSQRT: [u32; 9] = eight(0xfc00_002c);
const SIB_LHA: [u32; 9] = [
    0xaa3d_0000,
    0xaa5d_0002,
    0xaa7d_0004,
    0xaa9d_0006,
    0xaa3d_0000,
    0xaa5d_0002,
    0xaa7d_0004,
    0xaa9d_0006,
    BDNZ_8,
];

/// The dividend and divisor the probe's integer siblings start from.
const INT_SEEDS: &[Seed] = &[Seed::Gpr(15, 0x7fff_0000), Seed::Gpr(16, 1)];

/// The probe's siblings, phases 1 to 8.
pub(crate) const SIBLINGS: [(&str, Sibling); 8] = [
    (
        "addi",
        Sibling {
            words: &SIB_ADDI,
            seeds: &[],
        },
    ),
    (
        "divd",
        Sibling {
            words: &SIB_DIVD,
            seeds: INT_SEEDS,
        },
    ),
    (
        "divd independent",
        Sibling {
            words: &SIB_DIVD_IND,
            seeds: INT_SEEDS,
        },
    ),
    (
        "divw",
        Sibling {
            words: &SIB_DIVW,
            seeds: INT_SEEDS,
        },
    ),
    (
        "mulld",
        Sibling {
            words: &SIB_MULLD,
            seeds: INT_SEEDS,
        },
    ),
    (
        "fdiv",
        Sibling {
            words: &SIB_FDIV,
            seeds: &[Seed::Fpr(0, ONE_HALF), Seed::Fpr(12, ONE)],
        },
    ),
    (
        "fsqrt",
        Sibling {
            words: &SIB_FSQRT,
            seeds: &[Seed::Fpr(0, ONE)],
        },
    ),
    (
        "lha",
        Sibling {
            words: &SIB_LHA,
            seeds: &[Seed::Gpr(29, SELF_LINE + 0x80)],
        },
    ),
];

/// The largest share of a cell's mean console time the model may sit
/// from it, for a cell whose console runs spread over less.
const TOLERANCE: f64 = 0.20;

/// The cells the model times outside both the console's spread and
/// [`TOLERANCE`], as (sibling, chain):
///
/// - Independent `addi` beside `fdiv`. The model gives each `addi` 13
///   cycles and its own one, and times the loop 22% above the console's
///   mean. The `ppu_issue_units` capture runs the same loop beside the
///   same sibling within 19% of the model.
/// - Dependent `add` beside microcoded `lha`, 20% fast. Beside `lha` each
///   `add` waits about 14 cycles here, 9.5 in `ppu_issue_units`, 7.4 to
///   10.2 in `ppu_issue_layout`, and about 11 in the model.
const RESIDUALS: [(&str, &str); 2] = [("fdiv", "addi"), ("lha", "add")];

/// How far from the console's mean a residual may sit.
const RESIDUAL_LIMIT: f64 = 0.35;

#[test]
fn the_issue_model_times_each_chain_beside_each_sibling_near_the_console() {
    let w = console_words();
    let mut rows = Vec::new();
    for (p, (sibling, s)) in SIBLINGS.iter().enumerate() {
        for (c, chain) in CHAINS.iter().enumerate() {
            let cellgov = replay_beside(chain, 1, s).expect("the chain replays");
            let (fastest, slowest, mean) = console_cycles(&w, p + 1, c);
            rows.push((*sibling, NAMES[c], cellgov, fastest, slowest, mean));
        }
    }
    assert_eq!(rows.len(), 40);
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
fn a_sibling_of_long_instructions_costs_each_chain_more_than_one_of_multiplies() {
    // The console's order: a divide sibling costs every chain more than a
    // mulld sibling does, and a mulld sibling more than an addi one.
    let w = console_words();
    let [addi, divd, _, _, mulld, ..] = &SIBLINGS;
    for (c, chain) in CHAINS.iter().enumerate() {
        let cellgov = |s: &Sibling| replay_beside(chain, 1, s).expect("the chain replays");
        let console = |p: usize| console_cycles(&w, p, c).2;
        assert!(console(1) < console(5) && console(5) < console(2));
        let (a, m, d) = (cellgov(&addi.1), cellgov(&mulld.1), cellgov(&divd.1));
        assert!(
            a < m && m < d,
            "{}: CellGov beside addi {a:.2}, mulld {m:.2}, divd {d:.2}",
            NAMES[c],
        );
    }
}
