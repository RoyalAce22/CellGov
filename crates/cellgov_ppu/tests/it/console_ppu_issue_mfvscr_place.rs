//! CellGov's PPU timing against the `ppu_issue_mfvscr_place` console
//! capture: where in a 128-byte line an `mfvscr`, `li` and store loop
//! starts, and how many stores end it.
//!
//! The probe times 16 loops with `mftb`, five runs each.

use crate::console_ppu_issue_timing::{replay_at, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_mfvscr_place/ps3/cech20-cex-493/observation.json";

/// The loops the probe times.
const LOOPS: usize = 16;

/// The payload's last word when the probe wrote every word.
const DONE: u64 = 0x5050_4d50;

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

/// `mfvscr v3`.
const MFVSCR: u32 = 0x1060_0604;
/// `std r14, 0(r29)` to `std r14, 24(r29)`.
const STD: [u32; 4] = [0xf9dd_0000, 0xf9dd_0008, 0xf9dd_0010, 0xf9dd_0018];
/// `stfd f14, 0(r29)`.
const STFD: u32 = 0xd9dd_0000;
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

/// Each loop's body before its `bdnz`, and its offset from a 128-byte
/// boundary, in the probe's order.
fn loops() -> [(Vec<u32>, u64); LOOPS] {
    let held = |n, stores: &[u32]| [&[MFVSCR][..], &li(n), stores].concat();
    [
        (held(16, &[STFD]), 0),
        (held(16, &[STFD]), 32),
        (held(16, &[STFD]), 64),
        (held(16, &[STFD]), 96),
        (held(16, &STD[..1]), 0),
        (held(16, &STD[..1]), 32),
        (held(16, &STD[..1]), 64),
        (held(16, &STD[..1]), 96),
        (held(16, &STD[..2]), 0),
        (held(16, &STD[..3]), 0),
        (held(16, &STD[..4]), 0),
        (held(16, &STD[..2]), 64),
        (held(48, &STD[..1]), 0),
        (held(48, &STD[..1]), 64),
        (held(48, &[STFD]), 0),
        (held(48, &[STFD]), 64),
    ]
}

/// CellGov's cycles an iteration for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let (body, offset) = &loops()[k];
    let r = Replay {
        loop_index: k,
        words: looped(body),
        seeds: &[Seed::Gpr(29, ONE_LINE)],
    };
    replay_at(&r, *offset).expect("the loop replays")
}

/// The share of a loop's fastest console run the model may lie on either
/// side of it.
const BAND: f64 = 0.06;

/// The loops of 48 `li`, which the model gives fewer cycles than the
/// console. With an `std` the console takes 100.67 cycles an iteration
/// on a 128-byte boundary, and 86.81 at 64 bytes past one. With an `stfd`
/// it takes 106.04 and 106.05. The model gives 80 and 99.
///
/// In the `ppu_issue_mffs_far` capture 48 `li` alone take 56.77 cycles an
/// iteration, and the model gives 49.
const UNDER: [usize; 4] = [12, 13, 14, 15];

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
fn a_short_loop_costs_the_same_at_each_place_in_its_line() {
    // Loops 0 to 3 put the `stfd` loop, and loops 4 to 7 the `std` loop,
    // 0, 32, 64 and 96 bytes past a 128-byte boundary.
    let w = console_words();
    for (who, cycles) in [
        (
            "the console",
            &(|k| console_cycles(&w, k)) as &dyn Fn(usize) -> f64,
        ),
        ("CellGov", &cellgov_cycles),
    ] {
        for first in [0, 4] {
            let all: Vec<f64> = (first..first + 4).map(cycles).collect();
            let (lo, hi) = all
                .iter()
                .fold((f64::MAX, 0.0f64), |(lo, hi), &c| (lo.min(c), hi.max(c)));
            assert!(
                hi - lo < 2.0,
                "{who}: loops {first} to {} take {all:.2?} cycles an iteration",
                first + 3,
            );
        }
    }
}

/// The test fails when the model comes within the band of the console.
/// Then remove the loop from `UNDER`.
#[test]
fn each_48_li_loop_is_a_known_difference() {
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
