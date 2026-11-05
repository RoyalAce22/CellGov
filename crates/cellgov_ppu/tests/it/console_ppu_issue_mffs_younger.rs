//! CellGov's PPU timing against the `ppu_issue_mffs_younger` console
//! capture: when the vector-scalar instructions younger than an `mffs`
//! run, and how long an `mffs` waits for an older `fadd`, `fadd.` or
//! `vaddubm`.
//!
//! The probe times twelve loops with `mftb`, five runs each.

use crate::console_ppu_issue_timing::{replay, Replay};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_mffs_younger/ps3/cech20-cex-493/observation.json";

/// The loops the probe times.
const LOOPS: usize = 12;

/// The payload's word 26 when the probe wrote every word.
const DONE: u64 = 0x5050_4659;

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
/// `fadd f1, f14, f14` to `fadd f4, f14, f14`.
const FADD: [u32; 4] = [0xfc2e_702a, 0xfc4e_702a, 0xfc6e_702a, 0xfc8e_702a];
/// `fadd. f1, f14, f14`.
const FADD_RECORD: u32 = 0xfc2e_702b;
/// `vaddubm v4, v1, v1` to `vaddubm v7, v1, v1`.
const VADD: [u32; 4] = [0x1081_0800, 0x10a1_0800, 0x10c1_0800, 0x10e1_0800];
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

/// `n` `li`, cycling r3 to r10.
fn li(n: usize) -> Vec<u32> {
    LI.iter().copied().cycle().take(n).collect()
}

/// Each loop's body before its `bdnz`, in the probe's order.
fn bodies() -> [Vec<u32>; LOOPS] {
    let (li16, li32) = (li(16), li(32));
    [
        vec![MFFS],
        vec![MFFS, FADD[0]],
        [&[MFFS][..], &FADD].concat(),
        vec![MFFS, VADD[0]],
        [&[MFFS][..], &VADD].concat(),
        [&[MFFS][..], &li16].concat(),
        [&[MFFS][..], &li32].concat(),
        [&[MFFS, FADD[0]][..], &li16].concat(),
        [&[MFFS, VADD[0]][..], &li16].concat(),
        vec![FADD[0], MFFS],
        vec![FADD_RECORD, MFFS],
        vec![VADD[0], MFFS],
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

/// The loops the model holds: every loop but the `fadd.` before an `mffs`.
const HELD: [usize; 11] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 11];

#[test]
fn each_held_loop_lies_within_the_band() {
    let w = console_words();
    let misses: Vec<String> = HELD
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
fn a_vsu_op_right_after_an_mffs_waits_for_its_result() {
    // [CBE-Handbook p:722 s:A.1.2] an `mffs` stalls at VQ8 until older
    // vector-scalar instructions complete. An `fadd` or a `vaddubm` right
    // after it waits for its result, and the next `mffs` waits for that
    // instruction to complete: about twice the cycles of an `mffs` alone.
    let w = console_words();
    for (who, cycles) in [
        (
            "the console",
            &(|k| console_cycles(&w, k)) as &dyn Fn(usize) -> f64,
        ),
        ("CellGov", &cellgov_cycles),
    ] {
        let alone = cycles(0);
        for k in [1, 3] {
            assert!(
                cycles(k) > 1.8 * alone,
                "{who}: loop {k} takes {:.2} cycles an iteration, an mffs alone {alone:.2}",
                cycles(k),
            );
        }
    }
}

/// The loop of an `fadd.` before an `mffs`.
const RECORD_MFFS: usize = 10;

/// The console times this loop at 41.00 cycles an iteration, and a second
/// run of the probe at 50.45. The model gives 50. Both runs and the model
/// take longer than with an `fadd`.
#[test]
fn an_fadd_record_before_an_mffs_is_a_known_difference() {
    let w = console_words();
    let (cellgov, console) = (cellgov_cycles(RECORD_MFFS), console_cycles(&w, RECORD_MFFS));
    assert!(
        (cellgov - console) / console > BAND,
        "loop {RECORD_MFFS}: CellGov {cellgov:.2} cycles an iteration now lies within the band \
         of the console's {console:.2}; move the loop into HELD",
    );
    for (who, record, plain) in [
        ("the console", console, console_cycles(&w, 9)),
        ("CellGov", cellgov, cellgov_cycles(9)),
    ] {
        assert!(record > plain, "{who}: fadd. {record:.2}, fadd {plain:.2}");
    }
}
