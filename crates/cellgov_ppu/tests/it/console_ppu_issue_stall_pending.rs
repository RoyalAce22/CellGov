//! CellGov's PPU timing against the `ppu_issue_stall_pending` console
//! capture: a touch, a flush and a store-back, each with a load after
//! it, and an `mtvscr` with a `vaddubm` after it, alone and beside a
//! sibling of dependent `fdiv`.
//!
//! The sibling's divides stall the shared issue point back to back, and
//! the model holds each of the loop's dispatch groups the divide window
//! past the one before. Beside the sibling the console's runs of one loop
//! spread several-fold. The capture therefore does not settle whether the
//! divide window delays these:
//!
//! - a touched line's arrival;
//! - the fence an `mtvscr` sets;
//! - a line's busy time.

use crate::console_ppu_issue_sibling_rate::SIBLINGS;
use crate::console_ppu_issue_stall_kinds::SPAN_BAND;
use crate::console_ppu_issue_timing::{replay, replay_beside, Replay, Seed, Sibling};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_stall_pending/ps3/cech20-cex-493/observation.json";

/// The payload's word 106 when the probe wrote every word.
const DONE: u64 = 0x5050_5350;

/// The thread exit value the sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The probe's counted runs of each loop.
const REPS: u64 = 15;

/// Each timed loop's iterations on the console.
const ITERATIONS: u64 = 2048;

/// The probe's loops a phase.
const CHAINS: usize = 8;

/// The line r8 points at.
const LINE: u64 = 0x2000;

/// The start of the lines r7 walks, one a loop iteration.
const WALK: u64 = 0x1_0000;

/// The bytes r7 walks: room for every iteration a replay runs.
const WALK_BYTES: u64 = 0x4_0000;

/// `ld r9, 0(r7)`.
const LD_WALK: u32 = 0xe927_0000;
/// `ld r9, 0(r8)`.
const LD_LINE: u32 = 0xe928_0000;
/// `std r9, 0(r8)`.
const STD_LINE: u32 = 0xf928_0000;
/// `addi r7, r7, 128`.
const NEXT_LINE: u32 = 0x38e7_0080;
/// `dcbt 0, r7`.
const DCBT: u32 = 0x7c00_3a2c;
/// `dcbf 0, r8`.
const DCBF: u32 = 0x7c00_40ac;
/// `dcbst 0, r8`.
const DCBST: u32 = 0x7c00_406c;
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

/// The registers each loop starts with. r30 holds a ring that only sizes
/// the replay's memory to cover the lines r7 walks.
const SEEDS: &[Seed] = &[
    Seed::Gpr(7, WALK),
    Seed::Gpr(8, LINE),
    Seed::Gpr(9, 5),
    Seed::Ring(30, WALK, WALK_BYTES, false),
];

/// `body` closed by its `bdnz`.
fn looped(body: &[u32]) -> &'static [u32] {
    let mut words = body.to_vec();
    let back = (4 * words.len() as u32).wrapping_neg() & 0xfffc;
    words.push(0x4200_0000 | back);
    Box::leak(words.into_boxed_slice())
}

/// Each chain's body before its `bdnz`, in the probe's order.
fn bodies() -> [Vec<u32>; CHAINS] {
    [
        [&[LD_WALK][..], &ADDI8, &[NEXT_LINE]].concat(),
        [&[DCBT][..], &ADDI8, &[LD_WALK, NEXT_LINE]].concat(),
        [&[LD_LINE][..], &ADDI8].concat(),
        [&[DCBF][..], &ADDI8, &[LD_LINE]].concat(),
        [&[STD_LINE, DCBST][..], &ADDI8, &[LD_LINE]].concat(),
        [&[STD_LINE][..], &ADDI8, &[LD_LINE]].concat(),
        vec![MTVSCR, VADD],
        [&[MTVSCR][..], &ADDI8, &[VADD]].concat(),
    ]
}

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 856);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[106], DONE, "the probe wrote every word");
    assert_eq!((words[104], words[105]), (REPS, ITERATIONS));
    assert_eq!(
        (words[98], words[99], words[100], words[101], words[102]),
        (0, 0, SIBLING_EXIT, 1, 0),
        "the sibling's create rc, join rc, exit value, start and sleep rc",
    );
    for l in 0..2 * CHAINS {
        assert_eq!(
            words[64 + l],
            REPS,
            "loop {l} timed fifteen runs that count"
        );
    }
    words
}

/// Loop `chain`'s fastest counted run in phase `phase` on the console, in
/// cycles an iteration.
fn console_fastest(w: &[u64], phase: usize, chain: usize) -> f64 {
    let at = CHAINS * phase + chain;
    (w[2 * at] * PPU_CYCLES_PER_TB_TICK) as f64 / ITERATIONS as f64
}

fn sibling(name: &str) -> &'static Sibling {
    &SIBLINGS
        .iter()
        .find(|(n, _)| *n == name)
        .expect("a ppu_issue_sibling sibling")
        .1
}

/// The sibling's phase.
const FDIV: usize = 1;

/// Loop `chain`'s cycles an iteration in CellGov: alone in phase 0,
/// beside the `fdiv` sibling in phase 1.
fn cellgov(phase: usize, chain: usize) -> f64 {
    let r = Replay {
        loop_index: chain,
        words: looped(&bodies()[chain]),
        seeds: SEEDS,
    };
    let cycles = if phase == 0 {
        replay(&r, 0)
    } else {
        replay_beside(&r, 1, sibling("fdiv"))
    };
    cycles.unwrap_or_else(|| panic!("loop {chain} replays"))
}

/// The share of a loop's fastest console run alone the model may lie on
/// either side of it. The miss and flush loops lie up to 10% from the
/// console.
const BAND: f64 = 0.12;

/// The loops the model times under the console alone: the loop of a
/// store, a store-back of the same line and a load of it.
const UNDER: [usize; 1] = [4];

#[test]
fn alone_each_held_loop_lies_within_the_band() {
    let w = console_words();
    let held: Vec<usize> = (0..CHAINS).filter(|c| !UNDER.contains(c)).collect();
    assert_eq!(held.len(), CHAINS - UNDER.len());
    for c in held {
        let (cellgov, console) = (cellgov(0, c), console_fastest(&w, 0, c));
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "loop {c}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}",
        );
    }
}

#[test]
fn beside_the_sibling_each_loop_lies_within_its_runs() {
    // Beside the sibling a loop's runs spread up to 7.7 times, so the test
    // holds the model to the span of each loop's runs.
    let w = console_words();
    for c in 0..CHAINS {
        let at = CHAINS * FDIV + c;
        let run = |k: usize| (w[k] * PPU_CYCLES_PER_TB_TICK) as f64 / ITERATIONS as f64;
        let (lo, hi) = (run(2 * at), run(2 * at + 1));
        let cellgov = cellgov(FDIV, c);
        assert!(
            (1.0 - SPAN_BAND) * lo <= cellgov && cellgov <= (1.0 + SPAN_BAND) * hi,
            "loop {c}: CellGov {cellgov:.2} cycles an iteration, the console's runs {lo:.2} \
             to {hi:.2}",
        );
    }
}

/// The test fails when the model comes within the band of the console.
/// Then move the loop into the held set.
#[test]
fn alone_each_other_loop_is_a_known_difference() {
    let w = console_words();
    for c in UNDER {
        let (cellgov, console) = (cellgov(0, c), console_fastest(&w, 0, c));
        assert!(
            cellgov < (1.0 - BAND) * console,
            "loop {c}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}",
        );
    }
}
