//! CellGov's PPU timing against the `ppu_issue_mfvscr_runs` console
//! capture: how a run of independent VSU instructions after an `mfvscr`
//! issues, and when the next `mfvscr` issues after it.
//!
//! The probe times 14 loops with `mftb`, five runs each, on a 32-byte
//! boundary.

use crate::console_ppu_issue_timing::{replay, Replay};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_mfvscr_runs/ps3/cech20-cex-493/observation.json";

/// The loops the probe times.
const LOOPS: usize = 14;

/// The payload's last word when the probe wrote every word.
const DONE: u64 = 0x5050_5652;

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
/// `mfvscr v3`.
const MFVSCR3: u32 = 0x1060_0604;
/// `li r3, 1`.
const LI: u32 = 0x3860_0001;
/// `vaddubm v1, v2, v3`, then `vaddubm v5, v2, v3` to `vaddubm v11, v2, v3`.
const VADD: [u32; 8] = [
    0x1022_1800,
    0x10a2_1800,
    0x10c2_1800,
    0x10e2_1800,
    0x1102_1800,
    0x1122_1800,
    0x1142_1800,
    0x1162_1800,
];
/// `fadd f1, f14, f14` to `fadd f8, f14, f14`.
const FADD: [u32; 8] = [
    0xfc2e_702a,
    0xfc4e_702a,
    0xfc6e_702a,
    0xfc8e_702a,
    0xfcae_702a,
    0xfcce_702a,
    0xfcee_702a,
    0xfd0e_702a,
];
/// `vaddubm v4, v1, v1` to `vaddubm v7, v1, v1`.
const VADD_V1: [u32; 4] = [0x1081_0800, 0x10a1_0800, 0x10c1_0800, 0x10e1_0800];

/// `body` closed by its `bdnz`.
fn looped(body: &[u32]) -> &'static [u32] {
    let mut words = body.to_vec();
    let back = (4 * words.len() as u32).wrapping_neg() & 0xfffc;
    words.push(0x4200_0000 | back);
    Box::leak(words.into_boxed_slice())
}

/// Each body before its `bdnz`, in the probe's order.
fn bodies() -> [Vec<u32>; LOOPS] {
    let after = |run: &[u32]| [&[MFVSCR][..], run].concat();
    [
        vec![MFVSCR],
        after(&VADD[..1]),
        after(&VADD[..2]),
        after(&VADD[..3]),
        after(&VADD[..4]),
        after(&VADD[..6]),
        after(&VADD[..8]),
        after(&FADD[..2]),
        after(&FADD[..4]),
        after(&FADD[..8]),
        [&[MFVSCR3][..], &VADD_V1].concat(),
        VADD[..4].to_vec(),
        after(&[VADD[0], LI, VADD[1]]),
        after(&[VADD[0], FADD[0], VADD[1], FADD[1]]),
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

#[test]
fn each_loop_lies_within_the_band() {
    let w = console_words();
    let misses: Vec<String> = (0..LOOPS)
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
fn each_independent_vsu_op_after_an_mfvscr_adds_one_cycle() {
    // Loop 0 is an `mfvscr` alone. Loops 1 to 6 add 1, 2, 3, 4, 6 and 8
    // `vaddubm`, and loops 7 to 9 add 2, 4 and 8 `fadd`.
    let w = console_words();
    let runs: [(usize, f64); 9] = [
        (1, 1.0),
        (2, 2.0),
        (3, 3.0),
        (4, 4.0),
        (5, 6.0),
        (6, 8.0),
        (7, 2.0),
        (8, 4.0),
        (9, 8.0),
    ];
    for (who, cycles) in [
        (
            "the console",
            &(|k| console_cycles(&w, k)) as &dyn Fn(usize) -> f64,
        ),
        ("CellGov", &cellgov_cycles),
    ] {
        let alone = cycles(0);
        for (k, ops) in runs {
            let added = cycles(k) - alone;
            assert!(
                (added - ops).abs() < 0.5,
                "{who}: loop {k}, {ops} ops after an mfvscr, adds {added:.2} cycles an \
                 iteration to an mfvscr alone ({alone:.2})",
            );
        }
    }
}
