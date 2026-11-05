//! CellGov's PPU timing against the `ppu_issue_mffs_gap` console capture:
//! how the distance between an `mffs` and an `fadd`, in `li` on either
//! side and across a taken branch, sets the wait between them.
//!
//! The probe times sixteen loops with `mftb`, five runs each.

use crate::console_ppu_issue_timing::{replay, Replay};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_mffs_gap/ps3/cech20-cex-493/observation.json";

/// The loops the probe times.
const LOOPS: usize = 16;

/// The payload's word 34 when the probe wrote every word.
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

/// Each loop's body before its `bdnz`, in the probe's order.
fn bodies() -> [Vec<u32>; LOOPS] {
    let after = |n| [&[MFFS][..], &li(n), &[FADD]].concat();
    let before = |n| [&li(n)[..], &[FADD, MFFS]].concat();
    let between = |n| [&[FADD][..], &li(n), &[MFFS]].concat();
    [
        after(0),
        after(1),
        after(2),
        after(4),
        after(8),
        after(16),
        before(0),
        before(1),
        before(2),
        before(4),
        before(8),
        before(16),
        between(1),
        between(4),
        between(16),
        vec![MFFS, B, FADD],
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

/// The loops the model holds:
///
/// - an `fadd` right after an `mffs`, or 8 or 16 `li` after it;
/// - an `fadd` right before an `mffs`, or 2, 4 or 8 `li` and an `fadd`
///   before it;
/// - 4 or 16 `li` between an `fadd` and an `mffs`.
const HELD: [usize; 9] = [0, 4, 5, 6, 8, 9, 10, 13, 14];

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
fn only_the_op_right_after_an_mffs_waits_for_its_result() {
    // [CBE-Handbook p:757 s:A.6.2] IS2 sends instructions to the VSU
    // issue queue in pairs. An `fadd` right after an `mffs` waits for the
    // `mffs` result; with an `li` or a `b` between them it does not.
    let w = console_words();
    for (who, cycles) in [
        (
            "the console",
            &(|k| console_cycles(&w, k)) as &dyn Fn(usize) -> f64,
        ),
        ("CellGov", &cellgov_cycles),
    ] {
        let (paired, apart, branched) = (cycles(0), cycles(1), cycles(15));
        assert!(
            paired > 1.5 * apart && paired > 1.5 * branched,
            "{who}: {paired:.2} cycles an iteration paired, {apart:.2} with an li between, \
             {branched:.2} with a b between",
        );
    }
}

/// The side of the console's fastest run a known difference lies on.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Side {
    Under,
    Over,
}

/// The loops the model does not hold, each with the side of the console
/// it lies on:
///
/// - one `li`, two `li`, four `li` or a `b` between an `mffs` and a
///   younger `fadd`, and one `li` before or after an older one. The
///   console takes 33.00 to 35.00 cycles an iteration, 3 to 5 more than
///   an `mffs` alone, and the model gives 31 or 32. A second run of the
///   probe gave the same values.
/// - 16 `li` and an `fadd` before an `mffs`. The console takes 42.00
///   cycles an iteration and the model gives 46. With an `fadd` this far
///   from the older `mffs`, the younger `mffs` waits less after the `fadd`
///   than the model gives.
const DIFFERENT: [(usize, Side); 7] = [
    (1, Side::Under),
    (2, Side::Under),
    (3, Side::Under),
    (7, Side::Under),
    (12, Side::Under),
    (15, Side::Under),
    (11, Side::Over),
];

/// The test fails when the model comes within the band of the console,
/// or crosses to the other side. Then move the loop into `HELD`.
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
