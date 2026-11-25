//! CellGov's PPU timing against two console captures:
//!
//! - `ppu_issue_stall_kinds`: loops of `addi`, a hit, a store, a store and
//!   its reload, and an `mtvscr` with a `vaddubm`, each with 0, 2, 4 or 8
//!   `addi`;
//! - `ppu_issue_stall_runlen`: loops of `addi`, a hit and a store over
//!   three run lengths.
//!
//! Each loop runs alone and beside a sibling of dependent `fdiv`. Beside
//! that sibling the console's runs of one loop spread several-fold, within
//! a capture and between captures.

use crate::console_ppu_issue_sibling_rate::SIBLINGS;
use crate::console_ppu_issue_timing::{replay, replay_beside, Replay, Seed, Sibling};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const KINDS: &str = "../../tests/micro/ppu_issue_stall_kinds/ps3/cech20-cex-493/observation.json";
const RUNLEN: &str = "../../tests/micro/ppu_issue_stall_runlen/ps3/cech20-cex-493/observation.json";
const PENDING: &str =
    "../../tests/micro/ppu_issue_stall_pending/ps3/cech20-cex-493/observation.json";

/// The thread exit value each probe's sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The probes' counted runs of each loop.
const REPS: u64 = 15;

/// The `ppu_issue_stall_kinds` loops: five families of four.
const CHAINS: usize = 20;

/// Each `ppu_issue_stall_kinds` run's iterations.
const ITERATIONS: u64 = 2048;

/// The `ppu_issue_stall_runlen` run lengths.
const LENGTHS: [u64; 3] = [2048, 16384, 65536];

/// The line r8 points at.
const LINE: u64 = 0x2000;

/// `ld r9, 0(r8)`.
const LD_LINE: u32 = 0xe928_0000;
/// `std r9, 0(r8)`.
const STD_LINE: u32 = 0xf928_0000;
/// `mtvscr v0`.
const MTVSCR: u32 = 0x1000_0644;
/// `vaddubm v1, v2, v3`.
const VADD: u32 = 0x1022_1800;
/// `addi r17, r17, 1` to `addi r24, r24, 1`.
const ADDI8: [u32; 8] = [
    0x3a31_0001,
    0x3a52_0001,
    0x3a73_0001,
    0x3a94_0001,
    0x3ab5_0001,
    0x3ad6_0001,
    0x3af7_0001,
    0x3b18_0001,
];

/// The `addi` each family's loops carry, in chain order.
const K: [usize; 4] = [0, 2, 4, 8];

const SEEDS: &[Seed] = &[Seed::Gpr(8, LINE), Seed::Gpr(9, 5)];

/// The console's payload at `path` as big-endian words, `len` bytes.
fn words(path: &str, len: usize) -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), len);
    p.chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

/// The `ppu_issue_stall_kinds` payload, checked whole.
fn kinds_words() -> Vec<u64> {
    let w = words(KINDS, 2008);
    assert_eq!(w[250], 0x5050_534b, "the probe wrote every word");
    assert_eq!((w[248], w[249]), (REPS, ITERATIONS));
    assert_eq!(
        (w[242], w[243], w[244], w[245], w[246]),
        (0, 0, SIBLING_EXIT, 1, 0),
        "the sibling's create rc, join rc, exit value, start and sleep rc",
    );
    for l in 0..2 * CHAINS {
        assert_eq!(w[160 + l], REPS, "loop {l} timed fifteen runs that count");
    }
    w
}

/// The `ppu_issue_stall_runlen` payload, checked whole.
fn runlen_words() -> Vec<u64> {
    let w = words(RUNLEN, 1232);
    assert_eq!(w[153], 0x5050_524c, "the probe wrote every word");
    assert_eq!(w[152], REPS);
    assert_eq!(
        (w[146], w[147], w[148], w[149], w[150]),
        (0, 0, SIBLING_EXIT, 1, 0),
        "the sibling's create rc, join rc, exit value, start and sleep rc",
    );
    for l in 0..24 {
        assert_eq!(w[96 + l], REPS, "loop {l} timed fifteen runs that count");
    }
    w
}

/// Cycles an iteration of `ticks` over `iterations`.
fn cycles(ticks: u64, iterations: u64) -> f64 {
    (ticks * PPU_CYCLES_PER_TB_TICK) as f64 / iterations as f64
}

/// A `ppu_issue_stall_kinds` loop's fastest and slowest counted run, in
/// cycles an iteration: alone in phase 0, beside the sibling in phase 1.
fn kinds_runs(w: &[u64], phase: usize, chain: usize) -> (f64, f64) {
    let l = CHAINS * phase + chain;
    (
        cycles(w[2 * l], ITERATIONS),
        cycles(w[2 * l + 1], ITERATIONS),
    )
}

/// Chain `chain`'s body before its `bdnz`, in the probe's order.
fn body(chain: usize) -> Vec<u32> {
    let addi = &ADDI8[..K[chain % 4]];
    match chain / 4 {
        0 => addi.to_vec(),
        1 => [&[LD_LINE][..], addi].concat(),
        2 => [&[STD_LINE][..], addi].concat(),
        3 => [&[STD_LINE][..], addi, &[LD_LINE]].concat(),
        _ => [&[MTVSCR][..], addi, &[VADD]].concat(),
    }
}

/// `body` closed by its `bdnz`.
fn looped(body: &[u32]) -> &'static [u32] {
    let mut words = body.to_vec();
    let back = (4 * words.len() as u32).wrapping_neg() & 0xfffc;
    words.push(0x4200_0000 | back);
    Box::leak(words.into_boxed_slice())
}

fn fdiv_sibling() -> &'static Sibling {
    &SIBLINGS
        .iter()
        .find(|(n, _)| *n == "fdiv")
        .expect("a ppu_issue_sibling sibling")
        .1
}

/// Chain `chain`'s cycles an iteration in CellGov: alone in phase 0,
/// beside the `fdiv` sibling in phase 1.
fn cellgov(phase: usize, chain: usize) -> f64 {
    let r = Replay {
        loop_index: chain,
        words: looped(&body(chain)),
        seeds: SEEDS,
    };
    let cycles = if phase == 0 {
        replay(&r, 0)
    } else {
        replay_beside(&r, 1, fdiv_sibling())
    };
    cycles.unwrap_or_else(|| panic!("loop {chain} replays"))
}

/// A `ppu_issue_stall_kinds` loop's mean counted run beside the sibling,
/// in cycles an iteration.
fn kinds_mean_beside(w: &[u64], chain: usize) -> f64 {
    let l = CHAINS + chain;
    cycles(w[120 + l], ITERATIONS) / REPS as f64
}

/// The share of the console's fastest run alone the model may lie from it.
const ALONE_BAND: f64 = 0.10;

#[test]
fn alone_the_model_times_each_loop_near_the_console() {
    let w = kinds_words();
    for c in 0..CHAINS {
        let (console, _) = kinds_runs(&w, 0, c);
        let cellgov = cellgov(0, c);
        assert!(
            (cellgov - console).abs() / console <= ALONE_BAND,
            "loop {c} alone: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}",
        );
    }
}

/// The share of the `addi` loop's mean the hit and store loops lie within.
const KIND_SPREAD: f64 = 0.15;

#[test]
fn on_the_console_a_hit_or_a_store_beside_the_sibling_costs_what_an_addi_costs() {
    // The sibling's stall is not a stall per instruction kind.
    let w = kinds_words();
    for (k, &addi) in K.iter().enumerate() {
        let alone = kinds_mean_beside(&w, k);
        for (family, name) in [(1, "hit"), (2, "store")] {
            let mean = kinds_mean_beside(&w, 4 * family + k);
            assert!(
                (mean - alone).abs() / alone <= KIND_SPREAD,
                "{addi} addi: the {name} loop {mean:.2} cycles an iteration, the addi loop \
                 {alone:.2}",
            );
        }
    }
}

#[test]
fn on_the_console_the_same_loop_s_fastest_run_beside_the_sibling_moves_between_captures() {
    // Each index names the loop of an `ld` and 8 `addi` beside the sibling,
    // in three captures and at three run lengths.
    let pending = words(PENDING, 856);
    let kinds = kinds_words();
    let runlen = runlen_words();
    let mut fastest = vec![
        cycles(pending[2 * 10], ITERATIONS),
        kinds_runs(&kinds, 1, 7).0,
    ];
    for (i, &n) in LENGTHS.iter().enumerate() {
        fastest.push(cycles(runlen[2 * (12 + 6 + i)], n));
    }
    let lo = fastest.iter().copied().fold(f64::MAX, f64::min);
    let hi = fastest.iter().copied().fold(0.0, f64::max);
    assert!(hi > 3.0 * lo, "fastest runs {fastest:.2?}");
}

/// The share past the span of the console's runs the model may lie.
const SPAN_BAND: f64 = 0.12;

/// The `ppu_issue_stall_pending` loops beside the sibling that a
/// `ppu_issue_stall_kinds` loop repeats, as (pending chain, kinds chain):
///
/// - a hit and 8 `addi`;
/// - a store, 8 `addi` and a reload;
/// - an `mtvscr` and a `vaddubm`;
/// - an `mtvscr`, 8 `addi` and a `vaddubm`.
pub(crate) const REPEATED: [(usize, usize); 4] = [(2, 7), (5, 15), (6, 16), (7, 19)];

#[test]
fn beside_the_sibling_each_repeated_loop_lies_within_the_runs_both_captures_span() {
    let pending = words(PENDING, 856);
    let kinds = kinds_words();
    for (p, k) in REPEATED {
        let l = 8 + p;
        let (klo, khi) = kinds_runs(&kinds, 1, k);
        let lo = cycles(pending[2 * l], ITERATIONS).min(klo);
        let hi = cycles(pending[2 * l + 1], ITERATIONS).max(khi);
        let cellgov = cellgov(1, k);
        assert!(
            (1.0 - SPAN_BAND) * lo <= cellgov && cellgov <= (1.0 + SPAN_BAND) * hi,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's runs {lo:.2} \
             to {hi:.2}",
        );
    }
}

#[test]
fn beside_the_sibling_the_model_times_a_short_loop_short_of_the_console() {
    // The test pins an accepted gap. On the console the sibling's divides
    // set the loop's rate: a loop of one `bdnz`, or of an `ld` and a `bdnz`,
    // takes about a divide latency or more an iteration. The model adds a
    // capped stall to each instruction and times these loops at a quarter
    // of that or less.
    let w = kinds_words();
    for c in [0, 4] {
        let (console, _) = kinds_runs(&w, 1, c);
        let cellgov = cellgov(1, c);
        assert!(
            cellgov < 0.5 * console,
            "loop {c}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}",
        );
    }
}
