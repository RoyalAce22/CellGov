//! CellGov's PPU timing against the `ppu_issue_touch_set_forms` console
//! capture. The capture times a chase through memory in line order with
//! loads and touches of L2-held lines in front of each load:
//!
//! - loops 0 to 2: 0, 1 or 2 touches in the chase's L1 set;
//! - loop 3: a load in the chase's set;
//! - loop 4: a load and then a touch in the chase's set;
//! - loops 5 and 6: an `ldarx` chase after 4 touches in its set, and
//!   after none;
//! - loops 7 to 9: an `ld` in another set, across a 32-byte sector or in
//!   one, after 4 touches of that set or none.
//!
//! A load that misses the L1, an `ldarx` included, waits for its set and
//! then holds it as a touch does.
//!
//! The tests do not assert loops 7 to 9. On the console the chase waits
//! for the `ld` in the other set, and a sector-crossing `ld` that misses
//! the L1 costs more than CellGov charges.
//! `console_ppu_issue_touch_set_holds` and
//! `console_ppu_issue_touch_set_split` name both differences.

use crate::console_ppu_issue_timing::{replay, Replay, Seed};
use crate::console_ppu_issue_touch_set_pressure::{
    looped, CONFLICT, DCBT_R10, GROUP_BITS, IN_BUFFER, IN_GROUP, IN_ORDER_CHASE_GAP, LD, MEM_RING,
    NEXT_IN_SET, SET_BITS,
};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_touch_set_forms/ps3/cech20-cex-493/observation.json";

/// The loops the capture times.
const LOOPS: usize = 10;

/// The payload's last word when the probe wrote every word.
const DONE: u64 = 0x5050_5451;

/// log2 of each loop's iterations on the console, in each of the
/// `ppu_issue_touch_set_*` probes.
const ITERATIONS_LOG2: u32 = 12;

/// The runs of each loop, in each of the `ppu_issue_touch_set_*` probes.
const REPS: u64 = 5;

/// `ldarx r4, 0, r4`.
const LDARX: u32 = 0x7c80_20a8;
/// `ld r5, 0(r10)`.
pub(crate) const LD_R10: u32 = 0xe8aa_0000;

const SEEDS: &[Seed] = &[
    Seed::Ring(4, MEM_RING, 8 * 1024 * 1024, false),
    Seed::Gpr(20, CONFLICT),
];

/// The payload of the capture at `path`, as big-endian words, after a
/// check of its done word and its REPS and ITER words. A probe of `loops`
/// loops writes those three as its last words.
pub(crate) fn console_words(path: &str, loops: usize, done: u64) -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 8 * (2 * loops + 4), "{path}");
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(
        words[2 * loops + 3],
        done,
        "{path}: the probe wrote every word"
    );
    assert_eq!(
        (words[2 * loops + 1], words[2 * loops + 2]),
        (REPS, u64::from(ITERATIONS_LOG2)),
        "{path}"
    );
    words
}

/// Loop `k`'s fastest run, in core cycles an iteration.
pub(crate) fn console_cycles(words: &[u64], k: usize) -> f64 {
    (words[2 * k] * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64
}

/// The probe's prologue: r10 at the buffer line in r4's L1 set.
pub(crate) fn prologue() -> Vec<u32> {
    vec![SET_BITS, GROUP_BITS, IN_GROUP, IN_BUFFER]
}

/// `n` touches of buffer lines in r10's L1 set.
pub(crate) fn touches(n: usize) -> Vec<u32> {
    [DCBT_R10, NEXT_IN_SET].repeat(n)
}

/// Loop `k`'s body, in the probe's order, for the loops the tests replay.
fn body(k: usize) -> Vec<u32> {
    match k {
        0..=2 => [prologue(), touches(k), vec![LD]].concat(),
        3 => [prologue(), vec![LD_R10, NEXT_IN_SET, LD]].concat(),
        4 => [prologue(), vec![LD_R10, NEXT_IN_SET], touches(1), vec![LD]].concat(),
        5 => [prologue(), touches(4), vec![LDARX]].concat(),
        6 => [prologue(), vec![LDARX]].concat(),
        _ => unreachable!("loop {k} is not replayed"),
    }
}

/// CellGov's cycles a load for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let r = Replay {
        loop_index: k,
        words: looped(&body(k)),
        seeds: SEEDS,
    };
    replay(&r, 0).expect("the loop replays")
}

/// Asserts that each of `loops` replays within `IN_ORDER_CHASE_GAP` of
/// the console's fastest run, and returns CellGov's cycles and the
/// console's, in that order.
fn assert_within<const N: usize>(words: &[u64], loops: [usize; N]) -> [[f64; N]; 2] {
    let cellgov = loops.map(cellgov_cycles);
    let console = loops.map(|k| console_cycles(words, k));
    for ((k, g), c) in loops.iter().zip(cellgov).zip(console) {
        assert!(
            (c - g).abs() / c <= IN_ORDER_CHASE_GAP,
            "loop {k}: CellGov {g:.2} cycles a load, the console's fastest {c:.2}, \
             more than {IN_ORDER_CHASE_GAP} of it apart",
        );
    }
    [cellgov, console]
}

#[test]
fn a_load_that_misses_the_l1_holds_its_set_as_a_touch_does() {
    let w = console_words(CONSOLE, LOOPS, DONE);
    let sides = assert_within(&w, [0, 1, 2, 3, 4]);
    for (who, [none, one, two, load, load_touch]) in ["CellGov", "console"].into_iter().zip(sides) {
        let (one, two) = (one - none, two - none);
        let (load, load_touch) = (load - none, load_touch - none);
        assert!(
            (load - one).abs() <= 0.25 * one,
            "{who}: a load in the chase's set adds {load:.2} cycles, one touch there {one:.2}",
        );
        assert!(
            (load_touch - two).abs() <= 0.25 * two,
            "{who}: a load and a touch in the chase's set add {load_touch:.2} cycles, \
             two touches there {two:.2}",
        );
    }
}

#[test]
fn an_ldarx_waits_for_the_touch_reloads_its_set_holds() {
    let w = console_words(CONSOLE, LOOPS, DONE);
    let sides = assert_within(&w, [0, 1, 5, 6]);
    for (who, [none, one, four, ldarx]) in ["CellGov", "console"].into_iter().zip(sides) {
        let (one, four) = (one - none, four - ldarx);
        assert!(
            four > 3.0 * one,
            "{who}: 4 touches in an ldarx chase's set add {four:.2} cycles, \
             no more than 3 times the {one:.2} one touch adds to an ld chase",
        );
    }
}
