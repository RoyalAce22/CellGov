//! CellGov's PPU timing against the `ppu_issue_mffs_odd_target` console
//! capture: how a taken branch to an odd word sets the wait of an `fadd`
//! or a `vaddubm` after an `mffs` or an `mfvscr`. The probe times 14
//! loops with `mftb`, five runs each. A loop starts on a 32-byte boundary
//! or 4 bytes past one.

use crate::console_ppu_issue_timing::{replay_at, Replay};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_mffs_odd_target/ps3/cech20-cex-493/observation.json";

/// The loops the probe times.
const LOOPS: usize = 14;

/// The payload's last word when the probe wrote every word.
const DONE: u64 = 0x5050_465a;

/// log2 of each loop's iterations on the console: the probe's `ITER`.
const ITERATIONS_LOG2: u32 = 16;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let words: Vec<u64> = observation.memory_regions[0]
        .data
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words.len(), 2 * LOOPS + 3);
    assert_eq!(words[2 * LOOPS + 2], DONE, "the probe wrote every word");
    assert_eq!(
        (words[2 * LOOPS], words[2 * LOOPS + 1]),
        (5, u64::from(ITERATIONS_LOG2))
    );
    words
}

/// Loop `k`'s fastest run, in core cycles an iteration.
fn console_cycles(words: &[u64], k: usize) -> f64 {
    (words[2 * k] * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64
}

/// `mffs f0`.
const MFFS: u32 = 0xfc00_048e;
/// `mfvscr v0`.
const MFVSCR: u32 = 0x1000_0604;
/// `fadd f1, f14, f14`.
const FADD: u32 = 0xfc2e_702a;
/// `fadd f1, f1, f14`.
const DFADD: u32 = 0xfc21_702a;
/// `vaddubm v1, v2, v3`.
const VADD: u32 = 0x1022_1800;
/// `b` to the next instruction.
const B: u32 = 0x4800_0004;
/// `li r3, 1` to `li r10, 1`.
const LI: [u32; 8] = [
    0x3860_0001,
    0x3880_0001,
    0x38a0_0001,
    0x38c0_0001,
    0x38e0_0001,
    0x3900_0001,
    0x3920_0001,
    0x3940_0001,
];

/// `body` closed by its `bdnz`.
fn looped(body: &[u32]) -> &'static [u32] {
    let mut words = body.to_vec();
    let back = (4 * words.len() as u32).wrapping_neg() & 0xfffc;
    words.push(0x4200_0000 | back);
    Box::leak(words.into_boxed_slice())
}

/// Each body before its `bdnz`, in the probe's order. Loops `2i` and
/// `2i + 1` run body `i`: loop `2i` starts on a 32-byte boundary, and loop
/// `2i + 1` starts 4 bytes past one.
fn bodies() -> [Vec<u32>; LOOPS / 2] {
    [
        vec![DFADD],
        vec![DFADD, B, DFADD],
        vec![MFFS, B, LI[0], FADD],
        vec![MFFS, LI[0], B, FADD],
        [&[MFFS][..], &LI, &[B, FADD]].concat(),
        vec![MFFS, B, VADD],
        vec![MFVSCR, B, FADD],
    ]
}

/// CellGov's cycles an iteration for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let r = Replay {
        loop_index: k,
        words: looped(&bodies()[k / 2]),
        seeds: &[],
    };
    replay_at(&r, 4 * (k as u64 % 2)).expect("the loop replays")
}

/// The share of a loop's fastest console run the model may lie on either
/// side of it.
const BAND: f64 = 0.06;

/// The loops the model does not hold: an `mfvscr`, a `b` and an `fadd`,
/// with the `fadd` at an even word and at an odd word. The console takes
/// 42.00 cycles an iteration and the model gives 50.
const OVER: [usize; 2] = [12, 13];

#[test]
fn each_held_loop_lies_within_the_band() {
    let w = console_words();
    let held: Vec<usize> = (0..LOOPS).filter(|k| !OVER.contains(k)).collect();
    assert_eq!(held.len(), LOOPS - OVER.len());
    let misses: Vec<String> = held
        .into_iter()
        .map(|k| (k, cellgov_cycles(k), console_cycles(&w, k)))
        .filter(|&(_, cellgov, console)| (cellgov - console).abs() / console > BAND)
        .map(|(k, cellgov, console)| {
            format!(
                "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
                 {console:.2}"
            )
        })
        .collect();
    assert!(misses.is_empty(), "{}", misses.join("\n"));
}

#[test]
fn a_vsu_op_a_branch_lands_on_at_an_odd_word_waits_longer_after_an_mffs() {
    // Each pair is a loop with the branch target at an odd word, then the
    // same loop with the target at an even word. The targets are:
    // - an `fadd` after one `li` and a `b`;
    // - an `fadd` after eight `li` and a `b`;
    // - a `vaddubm` right after a `b`.
    let pairs = [(6, 7), (9, 8), (11, 10)];
    let w = console_words();
    for (who, cycles) in [
        (
            "the console",
            &(|k| console_cycles(&w, k)) as &dyn Fn(usize) -> f64,
        ),
        ("CellGov", &cellgov_cycles),
    ] {
        for (odd, even) in pairs {
            assert!(
                cycles(odd) >= cycles(even) + 2.5,
                "{who}: loop {odd} at an odd word {:.2} cycles an iteration, loop {even} at \
                 an even word {:.2}",
                cycles(odd),
                cycles(even),
            );
        }
    }
}

#[test]
fn an_fadd_chain_costs_the_same_at_an_odd_word_with_no_mffs() {
    let w = console_words();
    for (who, cycles) in [
        (
            "the console",
            &(|k| console_cycles(&w, k)) as &dyn Fn(usize) -> f64,
        ),
        ("CellGov", &cellgov_cycles),
    ] {
        for (even, odd) in [(0, 1), (2, 3)] {
            assert!(
                (cycles(odd) - cycles(even)).abs() < 0.5,
                "{who}: loop {even} {:.2} cycles an iteration, loop {odd} {:.2}",
                cycles(even),
                cycles(odd),
            );
        }
    }
}

/// The test fails when the model comes within the band of the console.
/// Then drop the loop from `OVER`.
#[test]
fn each_mfvscr_loop_is_a_known_difference() {
    let w = console_words();
    for k in OVER {
        let (cellgov, console) = (cellgov_cycles(k), console_cycles(&w, k));
        assert!(
            cellgov > (1.0 + BAND) * console,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}",
        );
    }
}
