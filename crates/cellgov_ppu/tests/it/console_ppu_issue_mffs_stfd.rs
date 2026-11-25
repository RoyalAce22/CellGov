//! CellGov's PPU timing against the `ppu_issue_mffs_stfd` console
//! capture: how far an `stfd` sits from an `mffs` sets the wait of the
//! next `mffs`.
//!
//! The probe times 20 loops with `mftb`, five runs each. A loop starts on
//! a 32-byte boundary or 4 bytes past one.

use crate::console_ppu_issue_timing::{replay_at, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_mffs_stfd/ps3/cech20-cex-493/observation.json";

/// The loops the probe times.
const LOOPS: usize = 20;

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
/// `stfd f14, 0(r11)`.
const STFD: u32 = 0xd9cb_0000;
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

/// Each loop's body before its `bdnz`, in the probe's order. Loop `2i`
/// starts on a 32-byte boundary and loop `2i + 1` 4 bytes past one.
fn bodies() -> [Vec<u32>; LOOPS / 2] {
    let after = |n| [&[MFFS][..], &li(n), &[STFD]].concat();
    let before = |n| [&li(n)[..], &[STFD, MFFS]].concat();
    [
        after(0),
        after(1),
        after(2),
        after(4),
        after(8),
        after(16),
        before(0),
        before(1),
        before(4),
        vec![MFFS],
    ]
}

/// CellGov's cycles an iteration for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let r = Replay {
        loop_index: k,
        words: looped(&bodies()[k / 2]),
        seeds: &[Seed::Gpr(11, ONE_LINE)],
    };
    replay_at(&r, 4 * (k as u64 % 2)).expect("the loop replays")
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
/// it lies on:
///
/// - `mffs; 4 li; stfd` on a 32-byte boundary: the console takes 35.00
///   cycles an iteration and the model gives 32.
/// - `mffs; 16 li; stfd` 4 bytes past one: 42.00 and 45.
/// - `4 li; stfd; mffs` 4 bytes past one: 35.33 and 33.
///
/// An `fadd` in place of the `stfd` lies the same way in the
/// `ppu_issue_mffs_gap` and `ppu_issue_mffs_align` captures. The model
/// times each `li` after an `mffs` at one cycle, and the console does not.
const DIFFERENT: [(usize, Side); 3] = [(6, Side::Under), (11, Side::Over), (17, Side::Under)];

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
fn an_stfd_waits_longer_paired_with_an_mffs_and_at_an_odd_word() {
    // Loop 0 pairs the `stfd` with the `mffs` and loop 1 does not. Loop
    // 13 lands the loop's branch on the `stfd` at an odd word and loop 12
    // at an even word.
    let w = console_words();
    for (who, cycles) in [
        (
            "the console",
            &(|k| console_cycles(&w, k)) as &dyn Fn(usize) -> f64,
        ),
        ("CellGov", &cellgov_cycles),
    ] {
        assert!(
            cycles(0) > 1.5 * cycles(1),
            "{who}: {:.2} cycles an iteration paired, {:.2} unpaired",
            cycles(0),
            cycles(1),
        );
        assert!(
            cycles(13) >= cycles(12) + 6.0,
            "{who}: {:.2} cycles an iteration at an odd word, {:.2} at an even word",
            cycles(13),
            cycles(12),
        );
    }
}

/// When the model times a loop within the band of the console, drop the
/// loop from `DIFFERENT`.
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
