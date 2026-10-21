//! CellGov's PPU timing against the `ppu_issue_loop_shape` console
//! capture: a sibling of dependent `divd` or independent `addi` whose
//! rounds run in one of four loop shapes, beside three chains.
//!
//! Shape B, the assembly loop with no time-base read, is the one the
//! replay runs: its words are the `ppu_issue_sibling` loop's, with the
//! stop flag in r8 and the round count in r7.

use crate::console_ppu_issue_sibling_rate::{
    cellgov_kept, same, sibling_loop, CHAINS_TIMED, COUNT, STOP, TOLERANCE,
};
use crate::console_ppu_issue_timing::{replay_beside, Seed, Sibling};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_loop_shape/ps3/cech20-cex-493/cgov_frame.bin";

/// The payload's word 164 when the probe wrote every word.
const DONE: u64 = 0x5050_534C;

/// The thread exit value each sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The probe's repetitions of each loop.
const REPS: u64 = 5;

/// Each timed loop's iterations on the console.
const ITERATIONS: u64 = 1 << 16;

/// The four loop shapes, in the probe's order.
const SHAPES: [&str; 4] = ["A", "B", "C", "D"];

/// The phase of a sibling of `divides` or not, in shape `shape`.
const fn phase(divides: bool, shape: usize) -> usize {
    if divides {
        1 + shape
    } else {
        5 + shape
    }
}

/// The chains the probe times, as indexes into [`CHAINS_TIMED`]: `add`,
/// `fadd` and `ld`.
const CHAINS: [usize; 3] = [0, 3, 4];

/// The share of its alone rate a sibling must keep in a cell for the
/// cell to count as one where the sibling held its hardware thread.
const HELD: f64 = 0.9;

const SIB_ADDI: [u32; 18] = sibling_loop(
    [
        0x3a31_0001,
        0x3a52_0001,
        0x3a73_0001,
        0x3a94_0001,
        0x3a31_0001,
        0x3a52_0001,
        0x3a73_0001,
        0x3a94_0001,
    ],
    8,
    7,
);
const SIB_DIVD: [u32; 18] = sibling_loop(same(0x7def_83d2), 8, 7);

const SEEDS: &[Seed] = &[
    Seed::Gpr(8, STOP),
    Seed::Gpr(7, COUNT),
    Seed::Gpr(15, 0x7fff_0000),
    Seed::Gpr(16, 1),
];

const ADDI: Sibling = Sibling {
    words: &SIB_ADDI,
    seeds: SEEDS,
};
const DIVD: Sibling = Sibling {
    words: &SIB_DIVD,
    seeds: SEEDS,
};

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let words: Vec<u64> = frame[8..]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words.len(), 168);
    assert_eq!(words[164], DONE, "the probe wrote every word");
    assert_eq!(words[163], REPS, "five runs a loop");
    for p in 1..=8 {
        assert_eq!(words[126 + p], 0, "phase {p}'s sibling was created");
        assert_eq!(words[135 + p], 0, "phase {p}'s sibling joined");
        assert_eq!(words[144 + p], SIBLING_EXIT, "phase {p}'s sibling exited");
        assert_eq!(words[153 + p], 1, "phase {p}'s sibling ran");
    }
    words
}

/// The share of its alone rate the sibling of `phase` keeps beside chain
/// `chain` on the console.
fn console_kept(w: &[u64], phase: usize, chain: usize) -> f64 {
    let at = CHAINS.len() * phase + chain;
    let rate = |rounds: u64, ticks: u64| rounds as f64 / ticks as f64;
    rate(w[54 + at], w[81 + at]) / rate(w[108 + 2 * phase], w[109 + 2 * phase])
}

/// Chain `chain`'s mean cycles an iteration beside the sibling of
/// `phase` on the console.
fn console_mean(w: &[u64], phase: usize, chain: usize) -> f64 {
    let at = CHAINS.len() * phase + chain;
    (w[81 + at] * PPU_CYCLES_PER_TB_TICK) as f64 / (REPS * ITERATIONS) as f64
}

#[test]
fn the_later_thread_gives_way_beside_a_dependent_chain_in_every_loop_shape() {
    // On the console the `addi` sibling keeps under a quarter of its rate
    // beside `fadd` and under two fifths beside `ld` in every shape: no
    // time-base read and no return to C between rounds lets it through.
    let w = console_words();
    let (fadd, ld) = (1, 2);
    for (s, shape) in SHAPES.iter().enumerate() {
        let p = phase(false, s);
        assert!(
            console_kept(&w, p, fadd) < 0.25,
            "shape {shape} beside fadd"
        );
        assert!(console_kept(&w, p, ld) < 0.4, "shape {shape} beside ld");
    }
    let b = phase(false, 1);
    for (c, &chain) in CHAINS.iter().enumerate() {
        let (name, replay) = &CHAINS_TIMED[chain];
        let (cellgov, console) = (cellgov_kept(replay, &ADDI), console_kept(&w, b, c));
        assert!(
            (console / cellgov - 1.0).abs() <= TOLERANCE,
            "shape B beside {name}: CellGov keeps {cellgov:.2} of the sibling's rate, the \
             console {console:.2}",
        );
    }
}

#[test]
fn beside_a_held_divide_sibling_the_model_waits_short_of_the_console() {
    // An accepted gap: beside a divide sibling that held its thread the
    // console's chains wait 19 to 30 cycles an instruction past their
    // alone time in this frame, and the level moves between runs of this
    // probe and between probes. The model's stall is the 13 that `ppu_issue_shared`,
    // `ppu_issue_units`, `ppu_issue_layout` and `ppu_issue_multiply` hold.
    let w = console_words();
    let mut held = 0;
    for (s, shape) in SHAPES.iter().enumerate() {
        let p = phase(true, s);
        for (c, &chain) in CHAINS.iter().enumerate() {
            if console_kept(&w, p, c) < HELD {
                continue;
            }
            held += 1;
            let (name, replay) = &CHAINS_TIMED[chain];
            let cellgov = replay_beside(replay, 1, &DIVD).expect("the chain replays");
            let console = console_mean(&w, p, c);
            assert!(
                cellgov < console && cellgov > 0.45 * console,
                "{name} beside divd shape {shape}: CellGov {cellgov:.1} cycles an iteration, \
                 the console's mean {console:.1}",
            );
        }
    }
    // A add, all of B and C, and D add and ld.
    assert_eq!(held, 9, "held divide cells");
}
