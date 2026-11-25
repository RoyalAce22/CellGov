//! CellGov's PPU timing against the `ppu_issue_mffs_align` console
//! capture: how the place of an `mffs` and an `fadd` in their 8-byte
//! pairs sets the wait between them. The probe times 32 loops with
//! `mftb`, five runs each. A loop starts on a 32-byte boundary or 4 bytes
//! past one.

use crate::console_ppu_issue_timing::{replay_at, Replay};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_mffs_align/ps3/cech20-cex-493/observation.json";

/// The loops the probe times.
const LOOPS: usize = 32;

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
/// `fadd f1, f14, f14`.
const FADD: u32 = 0xfc2e_702a;
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

/// `n` `li`, cycling r3 to r10.
fn li(n: usize) -> Vec<u32> {
    LI.iter().copied().cycle().take(n).collect()
}

/// Each loop's body before its `bdnz` and its offset from a 32-byte
/// boundary, in the probe's order.
fn loops() -> [(Vec<u32>, u64); LOOPS] {
    let after = |n| [&[MFFS][..], &li(n), &[FADD]].concat();
    let before = |n| [&li(n)[..], &[FADD, MFFS]].concat();
    let between = |n| [&[FADD][..], &li(n), &[MFFS]].concat();
    let alone = |n| [&[MFFS][..], &li(n)].concat();
    [
        (after(0), 4),
        (after(1), 4),
        (after(2), 4),
        (after(4), 4),
        (after(8), 4),
        (after(16), 4),
        (before(0), 4),
        (before(1), 4),
        (before(2), 4),
        (before(4), 4),
        (before(8), 4),
        (before(16), 4),
        (between(1), 4),
        (between(4), 4),
        (between(16), 4),
        (vec![MFFS, B, FADD], 4),
        (alone(0), 0),
        (alone(0), 4),
        (alone(16), 0),
        (alone(16), 4),
        (alone(4), 0),
        (alone(4), 4),
        (alone(1), 0),
        (alone(1), 4),
        (after(12), 0),
        (after(12), 4),
        (before(12), 0),
        (before(12), 4),
        (after(6), 0),
        (after(6), 4),
        (before(6), 0),
        (before(6), 4),
    ]
}

/// CellGov's cycles an iteration for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let (body, offset) = &loops()[k];
    let r = Replay {
        loop_index: k,
        words: looped(body),
        seeds: &[],
    };
    replay_at(&r, *offset).expect("the loop replays")
}

/// The share of a loop's fastest console run the model may lie on either
/// side of it.
const BAND: f64 = 0.06;

/// The side of the console's fastest run a known difference lies on.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Side {
    Under,
    Over,
}

/// The loops the model does not hold, each with the side of the console
/// it lies on. Each has 4 to 16 `li` between an `mffs` and the younger
/// `fadd`:
///
/// - `mffs; 16 li; fadd`, 4 bytes past a 32-byte boundary: the console
///   takes 42.00 cycles an iteration and the model gives 45.
/// - `4 li; fadd; mffs`, 4 bytes past: 35.33 and 33.
/// - `mffs; 12 li; fadd`, on the boundary: 43.00 and 40.
/// - `12 li; fadd; mffs`, 4 bytes past: 44.02 and 41.
/// - `mffs; 6 li; fadd`, on the boundary: 37.08 and 34.
/// - `6 li; fadd; mffs`, 4 bytes past: 38.49 and 35.
///
/// The model times each `li` after an `mffs` at one cycle. On the console
/// 12 and 16 `li` take the same cycles an iteration, and so do 6 and 8.
const DIFFERENT: [(usize, Side); 6] = [
    (5, Side::Over),
    (9, Side::Under),
    (24, Side::Under),
    (27, Side::Under),
    (28, Side::Under),
    (31, Side::Under),
];

#[test]
fn each_held_loop_lies_within_the_band() {
    let w = console_words();
    let held: Vec<usize> = (0..LOOPS)
        .filter(|k| DIFFERENT.iter().all(|&(d, _)| d != *k))
        .collect();
    assert_eq!(held.len(), LOOPS - DIFFERENT.len());
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
fn an_fadd_in_the_pair_after_an_mffs_does_not_wait_for_its_result() {
    // [CBE-Handbook p:757 s:A.6.2] IS2 sends instructions to the VSU
    // issue queue in pairs. With the `mffs` at an odd word, the `fadd`
    // right after it starts the next pair.
    let w = console_words();
    for (who, cycles) in [
        (
            "the console",
            &(|k| console_cycles(&w, k)) as &dyn Fn(usize) -> f64,
        ),
        ("CellGov", &cellgov_cycles),
    ] {
        let (next, apart) = (cycles(0), cycles(1));
        assert!(
            next < 1.1 * apart,
            "{who}: {next:.2} cycles an iteration with the fadd right after the mffs, \
             {apart:.2} with an li between",
        );
    }
}

/// The test fails when the model comes within the band of the console,
/// or crosses to the other side. Then drop the loop from `DIFFERENT`.
#[test]
fn each_other_loop_is_a_known_difference() {
    let w = console_words();
    for (k, side) in DIFFERENT {
        let (cellgov, console) = (cellgov_cycles(k), console_cycles(&w, k));
        let lies = if cellgov < (1.0 - BAND) * console {
            Some(Side::Under)
        } else if cellgov > (1.0 + BAND) * console {
            Some(Side::Over)
        } else {
            None
        };
        assert_eq!(
            lies,
            Some(side),
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}",
        );
    }
}
