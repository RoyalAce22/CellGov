//! CellGov's PPU timing against the `ppu_issue_load_hit_store_sibling`
//! console capture.
//!
//! The probe times four chains on the main thread:
//! - dependent `add`;
//! - independent `addi`;
//! - an `std` and `ld` chain whose loads hit the store queue;
//! - one such pair, then independent `addi`.
//!
//! It times each chain in three phases:
//! - alone;
//! - beside a sibling thread of independent `addi`;
//! - beside a sibling that runs its own `std` and `ld` chain.
//!
//! On the console a sibling whose loads hit the store queue costs the
//! main thread less than a sibling of `addi`. The issue model counts the
//! wait for the recycled load's data as a stall at the shared issue
//! point, so it orders the two siblings the other way. A test pins that
//! difference.

use crate::console_ppu_issue_shared::{eight, SIBLINGS};
use crate::console_ppu_issue_timing::{replay_measured, Replay, Seed, Sibling, BDNZ_8, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_load_hit_store_sibling/ps3/cech20-cex-493/observation.json";

/// Each timed loop's iterations on the console, from the probe's header.
const ITERATIONS: u64 = 1 << 16;

/// The probe's runs of each loop.
const REPS: u64 = 5;

/// The payload's word 68 when the probe wrote every word.
const DONE: u64 = 0x504c_5353;

/// The thread exit value each sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 552);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[68], DONE, "the probe wrote every word");
    assert_eq!(words[67], REPS, "five runs a loop");
    for p in 1..3 {
        assert_eq!(words[54 + p], 0, "phase {p}'s sibling was created");
        assert_eq!(words[57 + p], 0, "phase {p}'s sibling joined");
        assert_eq!(words[60 + p], SIBLING_EXIT, "phase {p}'s sibling exited");
        assert_eq!(words[63 + p], 1, "phase {p}'s sibling ran");
        for c in 0..4 {
            assert!(
                words[24 + 4 * p + c] > 0,
                "phase {p}'s sibling ran rounds beside chain {c}"
            );
        }
    }
    words
}

/// Loop `4 * phase + chain`'s cycles an iteration on the console: the
/// fastest run, the slowest, and the mean over its runs.
fn console_cycles(words: &[u64], phase: usize, chain: usize) -> (f64, f64, f64) {
    let at = 4 * phase + chain;
    let per = |tb: u64, iterations: u64| (tb * PPU_CYCLES_PER_TB_TICK) as f64 / iterations as f64;
    (
        per(words[2 * at], ITERATIONS),
        per(words[2 * at + 1], ITERATIONS),
        per(words[36 + at], REPS * ITERATIONS),
    )
}

/// `std r9,0(r7)` and `ld r9,0(r7)`: r7 is line A.
const STD_A: u32 = 0xf927_0000;
const LD_A: u32 = 0xe927_0000;

/// `addi r17..r20,1` in turn, as the probe lays them out.
const ADDI_4: [u32; 4] = [0x3a31_0001, 0x3a52_0001, 0x3a73_0001, 0x3a94_0001];

/// `head`, then the `addi` in turn up to `N - 1` words, then `bdnz`.
const fn addi_after<const N: usize>(head: &[u32], bdnz: u32) -> [u32; N] {
    let mut words = [0; N];
    let mut i = 0;
    while i < N - 1 {
        words[i] = if i < head.len() {
            head[i]
        } else {
            ADDI_4[(i - head.len()) % 4]
        };
        i += 1;
    }
    words[N - 1] = bdnz;
    words
}

const CHAIN_ADD: [u32; 9] = eight(0x7d29_7a14);
const CHAIN_ADDI: [u32; 17] = addi_after(&[], 0x4200_ffc0);
const CHAIN_LHS: [u32; 9] = [STD_A, LD_A, STD_A, LD_A, STD_A, LD_A, STD_A, LD_A, BDNZ_8];
const CHAIN_LHS_ADDI: [u32; 19] = addi_after(&[STD_A, LD_A], 0x4200_ffb8);

/// The probe's chains, in its order, as the built image lays them out.
const CHAINS: [Replay; 4] = [
    Replay {
        loop_index: 0,
        words: &CHAIN_ADD,
        seeds: &[Seed::Gpr(9, 1), Seed::Gpr(15, 3)],
    },
    Replay {
        loop_index: 1,
        words: &CHAIN_ADDI,
        seeds: &[],
    },
    Replay {
        loop_index: 2,
        words: &CHAIN_LHS,
        seeds: &[Seed::Gpr(7, ONE_LINE), Seed::Gpr(9, 1)],
    },
    Replay {
        loop_index: 3,
        words: &CHAIN_LHS_ADDI,
        seeds: &[Seed::Gpr(7, ONE_LINE), Seed::Gpr(9, 1)],
    },
];

const NAMES: [&str; 4] = ["add", "addi", "std ld", "std ld addi"];

/// `std r17,0(r4)` and `ld r17,0(r4)`: r4 is the sibling's own line.
const SIB_STD: u32 = 0xfa24_0000;
const SIB_LD: u32 = 0xea24_0000;
const SIB_LHS_WORDS: [u32; 9] = [
    SIB_STD, SIB_LD, SIB_STD, SIB_LD, SIB_STD, SIB_LD, SIB_STD, SIB_LD, BDNZ_8,
];

/// The sibling's line, apart from line A.
const SIBLING_LINE: u64 = ONE_LINE + 0x1000;

/// The phase 2 sibling: its `std` and `ld` chain.
const SIB_LHS: Sibling = Sibling {
    words: &SIB_LHS_WORDS,
    seeds: &[Seed::Gpr(4, SIBLING_LINE), Seed::Gpr(17, 1)],
};

/// The phase 1 sibling: independent `addi`.
fn sib_addi() -> &'static Sibling {
    let (name, sibling) = &SIBLINGS[0];
    assert_eq!(*name, "addi");
    sibling
}

/// The largest distance of the model from a loop's mean time on the
/// console, as a share of that mean, for a loop whose console runs spread
/// over less.
const TOLERANCE: f64 = 0.20;

#[test]
fn each_chain_alone_runs_near_the_console() {
    let w = console_words();
    let mut rows = Vec::new();
    for (c, chain) in CHAINS.iter().enumerate() {
        let cellgov = replay_measured(chain, 0, sib_addi())
            .expect("the chain replays")
            .cycles;
        let (fastest, slowest, mean) = console_cycles(&w, 0, c);
        rows.push((NAMES[c], cellgov, fastest, slowest, mean));
    }
    assert_eq!(rows.len(), 4);
    for &(chain, cellgov, fastest, slowest, mean) in &rows {
        let near =
            (fastest..=slowest).contains(&cellgov) || (cellgov - mean).abs() / mean <= TOLERANCE;
        assert!(
            near,
            "{chain} alone: CellGov {cellgov:.2} cycles an iteration, the console \
             {fastest:.2} to {slowest:.2}, mean {mean:.2}: {rows:?}",
        );
    }
}

/// The chains a sibling whose loads hit the store queue costs less than a
/// sibling of `addi` on the console: all but the `std` and `ld` chain,
/// whose own wait covers either sibling.
const CHEAPER_BESIDE_LHS: [usize; 3] = [0, 1, 3];

#[test]
fn a_store_queue_sibling_costs_less_than_an_addi_sibling_on_the_console_and_more_in_the_model() {
    let w = console_words();
    let mut rows = Vec::new();
    for &c in &CHEAPER_BESIDE_LHS {
        let chain = &CHAINS[c];
        let console_addi = console_cycles(&w, 1, c).2;
        let console_lhs = console_cycles(&w, 2, c).2;
        let cellgov = |s: &Sibling| {
            replay_measured(chain, 1, s)
                .expect("the chain replays")
                .cycles
        };
        let (cellgov_addi, cellgov_lhs) = (cellgov(sib_addi()), cellgov(&SIB_LHS));
        rows.push((
            NAMES[c],
            console_addi,
            console_lhs,
            cellgov_addi,
            cellgov_lhs,
        ));
    }
    assert_eq!(rows.len(), 3);
    for &(chain, console_addi, console_lhs, cellgov_addi, cellgov_lhs) in &rows {
        assert!(
            console_lhs < console_addi,
            "{chain}: the console beside the std and ld sibling {console_lhs:.2}, \
             beside addi {console_addi:.2}: {rows:?}",
        );
        // The model's known difference: it counts each sibling `std`'s wait
        // for the recycled `ld`'s data as a stall at the shared issue
        // point. This assertion goes red once the model orders the two
        // siblings as the console does.
        assert!(
            cellgov_lhs > cellgov_addi,
            "{chain}: the model now orders the siblings as the console does \
             (std and ld {cellgov_lhs:.2}, addi {cellgov_addi:.2}); assert the \
             console's order instead: {rows:?}",
        );
    }
}
