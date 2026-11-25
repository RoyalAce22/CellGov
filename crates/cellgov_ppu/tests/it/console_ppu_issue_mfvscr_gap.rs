//! CellGov's PPU timing against the `ppu_issue_mfvscr_gap` console
//! capture: which part of a younger instruction's wait after an `mfvscr`
//! follows the `mfvscr`'s issue, and which part follows its result.
//!
//! The probe times 18 loops with `mftb`, five runs each, on a 32-byte
//! boundary. Loop 0 is an `mfvscr` alone. Each other loop adds, after the
//! `mfvscr`:
//!
//! 1. a gap of up to 8 `li`, a `b`, or a `b` and one `li`;
//! 2. an `fadd`, a `vaddubm` or a second `mfvscr`.

use crate::console_ppu_issue_timing::{replay, Replay};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_mfvscr_gap/ps3/cech20-cex-493/observation.json";

/// The loops the probe times.
const LOOPS: usize = 18;

/// The payload's last word when the probe wrote every word.
const DONE: u64 = 0x5050_5647;

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

/// `mfvscr v0`.
const MFVSCR: u32 = 0x1000_0604;
/// `mfvscr v4`.
const MFVSCR2: u32 = 0x1080_0604;
/// `fadd f1, f14, f14`.
const FADD: u32 = 0xfc2e_702a;
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

/// The first `n` `li`.
fn li(n: usize) -> &'static [u32] {
    &LI[..n]
}

/// Each body before its `bdnz`, in the probe's order.
fn bodies() -> [Vec<u32>; LOOPS] {
    let after = |gap: &[u32], last: u32| [&[MFVSCR][..], gap, &[last]].concat();
    [
        vec![MFVSCR],
        after(li(0), FADD),
        after(li(1), FADD),
        after(li(2), FADD),
        after(li(4), FADD),
        after(&[B], FADD),
        after(li(0), VADD),
        after(li(1), VADD),
        after(li(2), VADD),
        after(li(4), VADD),
        after(&[B], VADD),
        after(li(0), MFVSCR2),
        after(li(1), MFVSCR2),
        after(li(2), MFVSCR2),
        after(li(4), MFVSCR2),
        after(&[B], MFVSCR2),
        after(&[B, LI[0]], FADD),
        after(li(8), FADD),
    ]
}

/// CellGov's cycles an iteration for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let r = Replay {
        loop_index: k,
        words: looped(&bodies()[k]),
        seeds: &[],
    };
    replay(&r, 0).expect("the loop replays")
}

/// The share of a loop's fastest console run the model may lie on either
/// side of it.
const BAND: f64 = 0.06;

/// The loops the model does not hold, each shorter in the model than on
/// the console:
///
/// - a second `mfvscr` after two `li`;
/// - a second `mfvscr` after a `b`.
///
/// The console takes 85.59 and 85.69 cycles an iteration, and the model
/// gives 80. After none, one or four `li`, the console takes 80.00 to
/// 80.56 cycles an iteration.
const UNDER: [usize; 2] = [13, 15];

#[test]
fn each_held_loop_lies_within_the_band() {
    let w = console_words();
    let held: Vec<usize> = (0..LOOPS).filter(|k| !UNDER.contains(k)).collect();
    assert_eq!(held.len(), LOOPS - UNDER.len());
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
fn a_vsu_op_right_after_an_mfvscr_runs_before_its_result_and_eight_li_between_add_seven_cycles() {
    // Loop 0 is an `mfvscr` alone. Loops 1 and 6 add an `fadd` or a
    // `vaddubm` right after it. Loop 17 puts 8 `li` before the `fadd`.
    let w = console_words();
    for (who, cycles) in [
        (
            "the console",
            &(|k| console_cycles(&w, k)) as &dyn Fn(usize) -> f64,
        ),
        ("CellGov", &cellgov_cycles),
    ] {
        let (alone, fadd, vadd, gap) = (cycles(0), cycles(1), cycles(6), cycles(17));
        assert!(
            (fadd - alone - 1.0).abs() < 0.5 && (vadd - alone - 1.0).abs() < 0.5,
            "{who}: an mfvscr alone {alone:.2} cycles an iteration, with an fadd {fadd:.2}, \
             with a vaddubm {vadd:.2}",
        );
        assert!(
            (gap - fadd - 7.0).abs() < 1.0,
            "{who}: an mfvscr and an fadd {fadd:.2} cycles an iteration, with 8 li between \
             {gap:.2}",
        );
    }
}

/// The test fails when the model comes within the band of the console.
/// Then drop the loop from `UNDER`.
#[test]
fn each_second_mfvscr_loop_outside_the_band_is_a_known_difference() {
    let w = console_words();
    for k in UNDER {
        let (cellgov, console) = (cellgov_cycles(k), console_cycles(&w, k));
        assert!(
            cellgov < (1.0 - BAND) * console,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}",
        );
    }
}
