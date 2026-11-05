//! CellGov's PPU timing against the `ppu_issue_vscr_smt_depth` console
//! capture: how a sibling thread's vector work, held behind its own
//! `mtvscr`, slows the main thread's `vaddubm`.
//!
//! The probe's sibling runs one loop that loads a stop flag and counts its
//! rounds in memory, with no time-base read. Five chains run on the main
//! thread alone and beside each sibling.

use crate::console_ppu_issue_sibling_rate::{COUNT, STOP};
use crate::console_ppu_issue_timing::{replay_beside, Replay, Seed, Sibling};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_vscr_smt_depth/ps3/cech20-cex-493/cgov_frame.bin";

/// The payload's word 132 when the probe wrote every word.
const DONE: u64 = 0x5050_5744;

/// The thread exit value each sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The chains a phase times.
const CHAINS: usize = 5;

/// The siblings, phases 1 to 4.
const PHASES: usize = 4;

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
    assert_eq!(words.len(), 136);
    assert_eq!(words[132], DONE, "the probe wrote every word");
    assert_eq!(words[131], 5, "five runs a loop");
    for p in 1..=PHASES {
        assert_eq!(words[110 + p], 0, "phase {p}'s sibling was created");
        assert_eq!(words[115 + p], 0, "phase {p}'s sibling joined");
        assert_eq!(words[120 + p], SIBLING_EXIT, "phase {p}'s sibling exited");
        assert_eq!(words[125 + p], 1, "phase {p}'s sibling ran");
    }
    words
}

/// Chain `chain`'s mean run in `phase`, in core cycles an iteration.
fn console_mean(w: &[u64], phase: usize, chain: usize) -> f64 {
    let at = CHAINS * phase + chain;
    (w[75 + at] * PPU_CYCLES_PER_TB_TICK) as f64 / 5.0 / (1u64 << ITERATIONS_LOG2) as f64
}

/// `vaddubm vk, v20, v21`.
const fn vadd(k: u32) -> u32 {
    0x1014_a800 | (k << 21)
}

/// `vaddubm v1, v1, v21`.
const VADD_DEPENDENT: u32 = 0x1021_a800;

/// `mtvscr v0`.
const MTVSCR: u32 = 0x1000_0644;

/// `n` independent `vaddubm`, cycling v1 to v`targets`.
fn independent(n: u32, targets: u32) -> Vec<u32> {
    (0..n).map(|i| vadd(1 + i % targets)).collect()
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

/// Each phase's sibling, phases 1 to 4: an `mtvscr` and 1, 3, 7 or 15
/// independent `vaddubm`.
fn siblings() -> [Sibling; PHASES] {
    [1, 3, 7, 15].map(|n| sibling(&[&[MTVSCR][..], &independent(n, 7)].concat()))
}

/// `body` closed by its `bdnz`.
fn looped(body: &[u32]) -> &'static [u32] {
    let mut words = body.to_vec();
    words.push(0x4200_0000 | ((4 * body.len() as u32).wrapping_neg() & 0xfffc));
    Box::leak(words.into_boxed_slice())
}

/// Each chain, in the probe's order.
fn chains() -> [Replay; CHAINS] {
    let chain = |k: usize, body: Vec<u32>| Replay {
        loop_index: k,
        words: looped(&body),
        seeds: &[],
    };
    [
        chain(0, independent(2, 8)),
        chain(1, independent(4, 8)),
        chain(2, independent(8, 8)),
        chain(3, independent(16, 8)),
        chain(4, vec![VADD_DEPENDENT; 8]),
    ]
}

/// CellGov's cycles an iteration of chain `chain` beside the sibling of
/// `phase`.
fn cellgov_beside(phase: usize, chain: usize) -> f64 {
    replay_beside(&chains()[chain], 1, &siblings()[phase - 1]).expect("the chain replays")
}

/// The share of the console's mean run a held cell may lie on either side
/// of it, as in `console_ppu_issue_vscr_smt`.
const BAND: f64 = 0.25;

/// The cells, as (phase, chain), the model does not time within
/// [`BAND`] of the console's mean run:
///
/// - the 2 and 4 `vaddubm` chains beside the siblings of an `mtvscr` and
///   1 or 3 `vaddubm`, the 8 `vaddubm` chain beside the second, and the 2
///   and 4 `vaddubm` chains beside the sibling of an `mtvscr` and 15: the
///   console takes 1.35 to 2.1 times the model's cycles;
/// - the 16 `vaddubm` chain beside the sibling of an `mtvscr` and one
///   `vaddubm`, and the 4 `vaddubm` chain beside the sibling of an
///   `mtvscr` and 7: the console takes 0.73 and 0.78 of the model's
///   cycles.
///
/// Each mean is of five runs, some of which the sibling most likely did
/// not share. The `ppu_issue_vscr_smt_runs` capture records each run.
/// There the 4 `vaddubm` chain beside the sibling of an `mtvscr` and 7
/// takes 34.24 to 40.64 cycles an iteration, and this mean is 23.57. Every
/// run of these two cells lies above the model:
///
/// - the 8 `vaddubm` chain beside the sibling of an `mtvscr` and 3;
/// - the 4 `vaddubm` chain beside the sibling of an `mtvscr` and 15.
///
/// When a difference closes, the test fails. Then move the cell into the
/// held set.
const DIFFERENCES: [(usize, usize); 9] = [
    (1, 0),
    (1, 1),
    (1, 3),
    (2, 0),
    (2, 1),
    (2, 2),
    (3, 1),
    (4, 0),
    (4, 1),
];

/// The cell this test does not time: the 8 `vaddubm` chain beside the
/// sibling of an `mtvscr` and 7 `vaddubm`, the loop pair
/// `console_ppu_issue_vscr_smt` times as its phase 3, chain 0. Here one of
/// its five runs takes 17.48 cycles an iteration, near the chain's 13.35
/// alone. The others reach 63.96, as the 55.24 to 65.22 of that capture
/// do. So the mean, 48.67, does not measure the chain beside the
/// sibling.
const UNTIMED: (usize, usize) = (3, 2);

/// Each cell beside a sibling but [`UNTIMED`], with CellGov's cycles and
/// the console's.
fn cells() -> Vec<(usize, usize, f64, f64)> {
    let w = console_words();
    (1..=PHASES)
        .flat_map(|p| (0..CHAINS).map(move |c| (p, c)))
        .filter(|&cell| cell != UNTIMED)
        .map(|(p, c)| (p, c, cellgov_beside(p, c), console_mean(&w, p, c)))
        .collect()
}

#[test]
fn each_held_chain_beside_a_sibling_held_at_its_fence_lies_within_the_band_of_the_mean_run() {
    let rows = cells();
    let held: Vec<_> = rows
        .iter()
        .filter(|(p, c, ..)| !DIFFERENCES.contains(&(*p, *c)))
        .collect();
    assert_eq!(held.len(), PHASES * CHAINS - DIFFERENCES.len() - 1);
    for &&(p, c, cellgov, console) in &held {
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "phase {p} chain {c}: CellGov {cellgov:.2} cycles an iteration, the console's \
             mean {console:.2}, more than {BAND} of it apart: {rows:?}",
        );
    }
}

#[test]
fn each_other_chain_beside_a_sibling_held_at_its_fence_is_a_known_difference() {
    let rows = cells();
    let differences: Vec<_> = rows
        .iter()
        .filter(|(p, c, ..)| DIFFERENCES.contains(&(*p, *c)))
        .collect();
    assert_eq!(differences.len(), DIFFERENCES.len());
    for &&(p, c, cellgov, console) in &differences {
        assert!(
            (cellgov - console).abs() / console > BAND,
            "phase {p} chain {c}: CellGov {cellgov:.2} cycles an iteration now lies within \
             {BAND} of the console's mean {console:.2}; move the cell into the held set",
        );
    }
}
