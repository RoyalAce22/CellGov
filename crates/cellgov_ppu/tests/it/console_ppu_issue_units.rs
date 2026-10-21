//! CellGov's PPU timing against the `ppu_issue_units` console capture.
//!
//! The probe times seven FXU, FP and load chains on the main thread,
//! alone and beside a sibling thread of dependent `divd`, dependent or
//! independent `fdiv`, microcoded `lha`, dependent `fadd` or dependent
//! `mulld`. The replay runs each chain beside one PPU unit that runs the
//! sibling's loop.

use crate::console_ppu_issue_shared::eight;
use crate::console_ppu_issue_timing::{replay_beside, Replay, Seed, Sibling, BDNZ_8};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_units/ps3/cech20-cex-493/cgov_frame.bin";

/// Each timed loop's iterations on the console, from the probe's header.
const ITERATIONS: u64 = 1 << 16;

/// The probe's repetitions of each loop.
const REPS: u64 = 5;

/// The payload's word 244 when the probe wrote every word.
const DONE: u64 = 0x5050_5554;

/// The thread exit value each sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The chains a phase times.
const CHAINS: usize = 7;

/// The siblings, phases 1 to 6.
const PHASES: usize = 6;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let words: Vec<u64> = frame[8..]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words.len(), 248);
    assert_eq!(words[244], DONE, "the probe wrote every word");
    assert_eq!(words[243], REPS, "five runs a loop");
    for p in 1..=PHASES {
        assert_eq!(words[214 + p], 0, "phase {p}'s sibling was created");
        assert_eq!(words[221 + p], 0, "phase {p}'s sibling joined");
        assert_eq!(words[228 + p], SIBLING_EXIT, "phase {p}'s sibling exited");
        assert_eq!(words[235 + p], 1, "phase {p}'s sibling ran");
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
        per(words[150 + at], REPS * ITERATIONS),
    )
}

/// `bdnz` back over sixteen instructions.
const BDNZ_16: u32 = 0x4200_ffc0;

/// `addi r17..r20,1` in turn.
const ADDI: [u32; 4] = [0x3a31_0001, 0x3a52_0001, 0x3a73_0001, 0x3a94_0001];
/// `fadd f0,f9,f10,f11 += f12` in turn.
const FADD: [u32; 4] = [0xfc00_602a, 0xfd29_602a, 0xfd4a_602a, 0xfd6b_602a];

/// Sixteen words: `pattern` repeated, then a `bdnz` over them.
const fn sixteen(pattern: [u32; 4]) -> [u32; 17] {
    let mut w = [BDNZ_16; 17];
    let mut i = 0;
    while i < 16 {
        w[i] = pattern[i % 4];
        i += 1;
    }
    w
}

const CHAIN_ADDI: [u32; 17] = sixteen(ADDI);
const CHAIN_ADD: [u32; 9] = eight(0x7d29_7a14);
const CHAIN_FADD: [u32; 9] = eight(FADD[0]);
const CHAIN_FADD_IND: [u32; 17] = sixteen(FADD);
const CHAIN_LD: [u32; 9] = eight(0xe929_0000);
const CHAIN_MIXED: [u32; 17] = [
    ADDI[0], FADD[0], ADDI[1], FADD[1], ADDI[2], FADD[2], ADDI[3], FADD[3], ADDI[0], FADD[0],
    ADDI[1], FADD[1], ADDI[2], FADD[2], ADDI[3], FADD[3], BDNZ_16,
];
const CHAIN_FADD_ADDI: [u32; 17] = [
    FADD[0], ADDI[0], FADD[0], ADDI[0], FADD[0], ADDI[0], FADD[0], ADDI[0], FADD[0], ADDI[0],
    FADD[0], ADDI[0], FADD[0], ADDI[0], FADD[0], ADDI[0], BDNZ_16,
];

/// 0.0, 1.0 and 1.5 as doubles.
const ZERO: u64 = 0;
const ONE: u64 = 0x3ff0_0000_0000_0000;
const ONE_HALF: u64 = 0x3ff8_0000_0000_0000;

/// A line whose first doubleword points at itself.
const SELF_LINE: u64 = 0x4000;

const FP_SEEDS: &[Seed] = &[
    Seed::Fpr(0, ONE),
    Seed::Fpr(9, ONE),
    Seed::Fpr(10, ONE),
    Seed::Fpr(11, ONE),
    Seed::Fpr(12, ZERO),
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
    ("addi", chain(0, &CHAIN_ADDI, &[])),
    (
        "add",
        chain(1, &CHAIN_ADD, &[Seed::Gpr(9, 1), Seed::Gpr(15, 3)]),
    ),
    ("fadd", chain(2, &CHAIN_FADD, FP_SEEDS)),
    ("fadd independent", chain(3, &CHAIN_FADD_IND, FP_SEEDS)),
    (
        "ld",
        chain(4, &CHAIN_LD, &[Seed::Ring(9, SELF_LINE, 128, false)]),
    ),
    ("addi and fadd", chain(5, &CHAIN_MIXED, FP_SEEDS)),
    ("fadd then addi", chain(6, &CHAIN_FADD_ADDI, FP_SEEDS)),
];

const SIB_DIVD: [u32; 9] = eight(0x7def_83d2);
const SIB_FDIV: [u32; 9] = eight(0xfc00_6024);
const SIB_FDIV_IND: [u32; 9] = [
    0xfc00_6024,
    0xfd29_6024,
    0xfd4a_6024,
    0xfd6b_6024,
    0xfc00_6024,
    0xfd29_6024,
    0xfd4a_6024,
    0xfd6b_6024,
    BDNZ_8,
];
const SIB_LHA: [u32; 9] = [
    0xaa3b_0000,
    0xaa5b_0002,
    0xaa7b_0004,
    0xaa9b_0006,
    0xaa3b_0000,
    0xaa5b_0002,
    0xaa7b_0004,
    0xaa9b_0006,
    BDNZ_8,
];
const SIB_FADD: [u32; 9] = eight(FADD[0]);
const SIB_MULLD: [u32; 9] = eight(0x7def_81d2);

const INT_SEEDS: &[Seed] = &[Seed::Gpr(15, 0x7fff_0000), Seed::Gpr(16, 1)];
const FDIV_SEEDS: &[Seed] = &[
    Seed::Fpr(0, ONE_HALF),
    Seed::Fpr(9, ONE_HALF),
    Seed::Fpr(10, ONE_HALF),
    Seed::Fpr(11, ONE_HALF),
    Seed::Fpr(12, ONE),
];

/// The probe's siblings, phases 1 to 6.
const SIBLINGS: [(&str, Sibling); PHASES] = [
    (
        "divd",
        Sibling {
            words: &SIB_DIVD,
            seeds: INT_SEEDS,
        },
    ),
    (
        "fdiv",
        Sibling {
            words: &SIB_FDIV,
            seeds: FDIV_SEEDS,
        },
    ),
    (
        "fdiv independent",
        Sibling {
            words: &SIB_FDIV_IND,
            seeds: FDIV_SEEDS,
        },
    ),
    (
        "lha",
        Sibling {
            words: &SIB_LHA,
            seeds: &[Seed::Gpr(27, SELF_LINE + 0x80)],
        },
    ),
    (
        "fadd",
        Sibling {
            words: &SIB_FADD,
            seeds: &[Seed::Fpr(0, ONE), Seed::Fpr(12, ZERO)],
        },
    ),
    (
        "mulld",
        Sibling {
            words: &SIB_MULLD,
            seeds: INT_SEEDS,
        },
    ),
];

/// Each chain's CellGov cycles an iteration beside each sibling, with
/// the console's fastest, slowest and mean.
fn rows() -> Vec<(&'static str, &'static str, f64, f64, f64, f64)> {
    let w = console_words();
    let mut rows = Vec::new();
    for (p, (sibling, s)) in SIBLINGS.iter().enumerate() {
        for (c, (chain, r)) in CHAINS_TIMED.iter().enumerate() {
            let cellgov = replay_beside(r, 1, s).expect("the chain replays");
            let (fastest, slowest, mean) = console_cycles(&w, p + 1, c);
            rows.push((*sibling, *chain, cellgov, fastest, slowest, mean));
        }
    }
    rows
}

/// The largest share of a cell's mean console time the model may sit
/// from it, for a cell whose console runs spread over less.
const TOLERANCE: f64 = 0.20;

/// The cells the model times outside both the console's spread and
/// [`TOLERANCE`], as (sibling, chain):
///
/// - Three chains beside microcoded `lha`, which the model times 21% to
///   33% slow. Beside `lha` each instruction of an FXU or load chain
///   waits 7.1 to 9.5 cycles in this capture, each `add` 7.4 to 10.2 in
///   `ppu_issue_layout`, and each FXU or load instruction 12.5 to 14.4 in
///   `ppu_issue_shared`. The model caps that stall at 10 cycles.
/// - Four chains beside dependent `mulld`, which the model times 24% to
///   36% fast. Beside `mulld` each `addi` waits about 8 cycles here and
///   about 6 in `ppu_issue_shared`, and each `add` about 6 in
///   `ppu_issue_layout`.
/// - Interleaved `addi` and `fadd` beside `fdiv`, 22% slow.
const RESIDUALS: [(&str, &str); 8] = [
    ("lha", "addi"),
    ("lha", "ld"),
    ("lha", "fadd then addi"),
    ("mulld", "addi"),
    ("mulld", "add"),
    ("mulld", "fadd independent"),
    ("mulld", "addi and fadd"),
    ("fdiv", "addi and fadd"),
];

/// How far from the console's mean a residual may sit: just above the
/// worst residual, independent `fadd` beside `mulld` at 36%.
const RESIDUAL_LIMIT: f64 = 0.40;

#[test]
fn the_issue_model_times_each_chain_beside_each_sibling_near_the_console() {
    let rows = rows();
    assert_eq!(rows.len(), PHASES * CHAINS);
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
fn beside_microcode_or_multiplies_an_fp_chain_loses_less_than_an_fxu_chain() {
    // Cycles each instruction of the dependent add and fadd chains loses
    // beside the sibling, on the console and in the model. Beside lha and
    // mulld the fadd loses less; beside divd and fdiv about as much.
    let w = console_words();
    let per = |p: usize, c: usize, n: f64| {
        let console = (console_cycles(&w, p + 1, c).2 - console_cycles(&w, 0, c).2) / n;
        let alone = crate::console_ppu_issue_timing::replay(&CHAINS_TIMED[c].1, 0)
            .expect("the chain replays");
        let beside = replay_beside(&CHAINS_TIMED[c].1, 1, &SIBLINGS[p].1).expect("replays");
        (console, (beside - alone) / n)
    };
    for (p, &(name, _)) in SIBLINGS.iter().enumerate() {
        let (add, fadd) = (per(p, 1, 8.0), per(p, 2, 8.0));
        if matches!(name, "lha" | "mulld") {
            assert!(
                fadd.0 < 0.7 * add.0,
                "console beside {name}: add {add:?}, fadd {fadd:?}"
            );
            assert!(
                fadd.1 < 0.7 * add.1,
                "CellGov beside {name}: add {add:?}, fadd {fadd:?}"
            );
        } else if name != "fadd" {
            assert!(
                fadd.0 > 0.8 * add.0,
                "console beside {name}: add {add:?}, fadd {fadd:?}"
            );
            assert!(
                fadd.1 > 0.8 * add.1,
                "CellGov beside {name}: add {add:?}, fadd {fadd:?}"
            );
        }
    }
}

#[test]
fn a_dependent_fadd_with_an_addi_after_each_waits_both_stalls_beside_a_divide() {
    // The fadd chain alone takes its latencies with each addi hidden
    // between. Beside fdiv the console gives every instruction the
    // stall, the addi's included, on top of the fadd's latency.
    let w = console_words();
    let fdiv = 1;
    let (fastest, slowest, mean) = console_cycles(&w, fdiv + 1, 6);
    let alone = console_cycles(&w, 0, 6).2;
    assert!(
        (mean - alone) / 16.0 > 12.0,
        "console {mean:.1}, alone {alone:.1}"
    );
    let cellgov = replay_beside(&CHAINS_TIMED[6].1, 1, &SIBLINGS[fdiv].1).expect("replays");
    assert!(
        (fastest..=slowest).contains(&cellgov) || (cellgov - mean).abs() / mean <= TOLERANCE,
        "CellGov {cellgov:.1}, the console {fastest:.1} to {slowest:.1}, mean {mean:.1}",
    );
}
