//! CellGov's PPU timing against the `ppu_issue_stall_pending` console
//! capture: a touch, a flush and a store-back, each with a load after
//! it, and an `mtvscr` with a `vaddubm` after it, alone and beside a
//! sibling of dependent `fdiv`.
//!
//! The capture settles whether a sibling's stall delays a touched line's
//! arrival and the fence an `mtvscr` sets. Its flush and store-back loops
//! do not settle whether the stall delays a line's busy time.

use crate::console_ppu_issue_sibling_rate::SIBLINGS;
use crate::console_ppu_issue_stall_kinds::REPEATED;
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

/// The share of a loop's fastest console run the model may lie on either
/// side of it. The miss and flush loops lie up to 10% from the console.
const BAND: f64 = 0.12;

/// The loop the band holds most tightly: a touch, 8 `addi` and a load of
/// the touched line, beside the sibling.
const TOUCH_BESIDE: (usize, usize) = (FDIV, 1);

/// The share of the console's fastest run the touch loop beside the
/// sibling may lie from it.
const TOUCH_BAND: f64 = 0.06;

/// The cells, as (phase, chain), the model times under the console.
///
/// Both cells are the loop of a store, a store-back of the same line and a
/// load of it, alone and beside the sibling.
const UNDER: [(usize, usize); 2] = [(0, 4), (FDIV, 4)];

/// The cells beside the sibling whose loop the `ppu_issue_stall_kinds`
/// capture repeats.
///
/// There the same loop's fastest run differs from this capture's by up to
/// 3.3 times. A band about one capture's fastest run therefore does not
/// hold them. `console_ppu_issue_stall_kinds` holds each within the runs
/// both captures span.
fn repeated() -> impl Iterator<Item = (usize, usize)> {
    REPEATED.iter().map(|&(chain, _)| (FDIV, chain))
}

/// Each cell, with CellGov's cycles and the console's fastest run.
fn cells() -> Vec<(usize, usize, f64, f64)> {
    let w = console_words();
    (0..2)
        .flat_map(|p| (0..CHAINS).map(move |c| (p, c)))
        .map(|(p, c)| (p, c, cellgov(p, c), console_fastest(&w, p, c)))
        .collect()
}

#[test]
fn each_held_loop_lies_within_the_band() {
    let rows = cells();
    let held: Vec<_> = rows
        .iter()
        .filter(|(p, c, ..)| !UNDER.contains(&(*p, *c)) && !repeated().any(|r| r == (*p, *c)))
        .collect();
    assert_eq!(held.len(), 2 * CHAINS - UNDER.len() - REPEATED.len());
    for &&(p, c, cellgov, console) in &held {
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "phase {p} loop {c}: CellGov {cellgov:.2} cycles an iteration, the console's \
             fastest {console:.2}",
        );
    }
}

#[test]
fn beside_a_stalling_sibling_a_load_waits_for_a_touched_line_the_stalls_push_back() {
    // The 8 `addi` between the touch and the load stall beside the
    // sibling. On the console the loop then takes 385.25 cycles an
    // iteration, 172 more than alone. A touched line whose arrival does
    // not move is in the cache sooner, and the model then gives 320.
    let (p, c) = TOUCH_BESIDE;
    let w = console_words();
    let (cellgov, console) = (cellgov(p, c), console_fastest(&w, p, c));
    assert!(
        (cellgov - console).abs() / console <= TOUCH_BAND,
        "CellGov {cellgov:.2} cycles an iteration, the console's fastest {console:.2}",
    );
}

#[test]
fn beside_a_stalling_sibling_eight_addi_after_an_mtvscr_cost_what_a_fixed_fence_leaves() {
    // Beside the sibling an `mtvscr`, 8 `addi` and a `vaddubm` take 88.69
    // cycles an iteration more than an `mtvscr` and a `vaddubm`. With a
    // fence that the stalls delay, the model gives 103.89.
    let w = console_words();
    let console = console_fastest(&w, FDIV, 7) - console_fastest(&w, FDIV, 6);
    let cellgov = cellgov(FDIV, 7) - cellgov(FDIV, 6);
    assert!(
        (cellgov - console).abs() < 5.0,
        "the 8 addi after an mtvscr cost CellGov {cellgov:.2} cycles an iteration, the \
         console {console:.2}",
    );
}

/// The test fails when the model comes within the band of the console.
/// Then move the cell into the held set.
#[test]
fn each_other_loop_is_a_known_difference() {
    let rows = cells();
    let find = |p: usize, c: usize| {
        *rows
            .iter()
            .find(|row| (row.0, row.1) == (p, c))
            .expect("the cell")
    };
    for (p, c) in UNDER {
        let (_, _, cellgov, console) = find(p, c);
        assert!(
            cellgov < (1.0 - BAND) * console,
            "phase {p} loop {c}: CellGov {cellgov:.2} cycles an iteration, the console's \
             fastest {console:.2}",
        );
    }
}
