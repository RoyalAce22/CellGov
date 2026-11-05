//! CellGov's PPU timing against the `ppu_issue_quickened_forms` console
//! capture.
//!
//! The probe times five loops with `mftb`, five runs each. Each loop
//! replays through the execution unit twice:
//!
//! - from its raw words;
//! - from a predecoded shadow that quickens `ori r0,r0,0` to `nop` and
//!   fuses each compare with its branch.
//!
//! Both replays hold the loop's cycles an iteration no farther from the
//! console's fastest run than the slowest run is, or than `TOLERANCE` of
//! the fastest run.

use crate::console_ppu_issue_timing::{replay, replay_shadowed, Replay, Seed};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_quickened_forms/ps3/cech20-cex-493/observation.json";

/// log2 of each loop's iterations on the console, from the probe's
/// header.
const ITERATIONS_LOG2: [u32; 5] = [18, 18, 20, 20, 20];

/// The payload's word 12 when the probe wrote every word.
const DONE: u64 = 0x5050_5146;

/// The largest share of a loop's fastest console run the model may differ
/// by, past the run's own spread.
///
/// Three captures of the probe move a compare-and-branch loop's fastest
/// run by up to 16%.
const TOLERANCE: f64 = 0.20;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 128);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[12], DONE, "the probe wrote every word");
    assert_eq!(words[11], 5, "five runs a loop");
    words
}

/// Loop `k`'s fastest and slowest console runs, in core cycles an
/// iteration.
fn console_cycles(words: &[u64], k: usize) -> (f64, f64) {
    let per = |tb: u64| (tb * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2[k]) as f64;
    (per(words[2 * k]), per(words[2 * k + 1]))
}

/// `add r9,r9,r15`.
const ADD: u32 = 0x7d29_7a14;
/// `ori r9,r9,0`.
const ORI_R9: u32 = 0x6129_0000;
/// `ori r0,r0,0`, the preferred no-op.
const NOP: u32 = 0x6000_0000;
/// `cmpwi cr0,r14,0`.
const CMPWI: u32 = 0x2c0e_0000;
/// `cmpw cr0,r14,r15`.
const CMPW: u32 = 0x7c0e_7800;
/// `beq` to the next instruction.
const BEQ_NEXT: u32 = 0x4182_0004;
/// `bne` to the next instruction.
const BNE_NEXT: u32 = 0x4082_0004;
/// The `bdnz` that closes a 16-instruction body.
const BDNZ_16: u32 = 0x4200_ffc0;

/// Eight copies of `a; b`, then the loop's `bdnz`.
const fn eight_pairs(a: u32, b: u32) -> [u32; 17] {
    [a, b, a, b, a, b, a, b, a, b, a, b, a, b, a, b, BDNZ_16]
}

const ADD_SEEDS: &[Seed] = &[Seed::Gpr(9, 1), Seed::Gpr(15, 3)];
const CMP_SEEDS: &[Seed] = &[Seed::Gpr(14, 0), Seed::Gpr(15, 0)];

const REPLAYS: [Replay; 5] = [
    Replay {
        loop_index: 0,
        words: &eight_pairs(ADD, ORI_R9),
        seeds: ADD_SEEDS,
    },
    Replay {
        loop_index: 1,
        words: &eight_pairs(ADD, NOP),
        seeds: ADD_SEEDS,
    },
    Replay {
        loop_index: 2,
        words: &eight_pairs(CMPWI, BEQ_NEXT),
        seeds: CMP_SEEDS,
    },
    Replay {
        loop_index: 3,
        words: &eight_pairs(CMPWI, BNE_NEXT),
        seeds: CMP_SEEDS,
    },
    Replay {
        loop_index: 4,
        words: &eight_pairs(CMPW, BEQ_NEXT),
        seeds: CMP_SEEDS,
    },
];

/// Hold each of `loops` within the console's spread, run from raw words
/// and from a predecoded shadow.
fn assert_within_spread(loops: &[usize]) {
    let w = console_words();
    let mut rows = Vec::new();
    for &k in loops {
        let raw = replay(&REPLAYS[k], 0).expect("the loop replays");
        let shadowed = replay_shadowed(&REPLAYS[k]).expect("the shadowed loop replays");
        let (fastest, slowest) = console_cycles(&w, k);
        rows.push((k, raw, shadowed, fastest, slowest));
    }
    for &(k, raw, shadowed, fastest, slowest) in &rows {
        for (form, cellgov) in [("raw", raw), ("shadowed", shadowed)] {
            assert!(
                (cellgov - fastest).abs() <= (slowest - fastest).max(TOLERANCE * fastest),
                "loop {k} {form}: CellGov {cellgov:.3} cycles an iteration, the console \
                 {fastest:.3} to {slowest:.3}: {rows:?}",
            );
        }
    }
}

/// `ori r9,r9,0` waits for r9 like an `add` does: the chain takes about
/// 32 cycles an iteration with it and 17 with `nop`.
#[test]
fn ori_rn_rn_0_in_a_chain_times_within_the_console_s_spread() {
    assert_within_spread(&[0, 1]);
}

/// A conditional branch taken to the next instruction after `cmpwi` or
/// `cmpw` costs what one that falls through costs.
#[test]
fn a_compare_and_branch_to_the_next_instruction_times_within_the_console_s_spread() {
    assert_within_spread(&[2, 3, 4]);
}
