//! CellGov's PPU timing against the `ppu_issue_mfvscr_younger` console
//! capture: when the instructions younger than an `mfvscr` issue, and how
//! fast a run of `li` issues with and without a write to one register.
//!
//! The probe times fourteen loops with `mftb`, five runs each. Every
//! store addresses one data line, which r29 holds.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_mfvscr_younger/ps3/cech20-cex-493/observation.json";

/// The payload's word 30 when the probe wrote every word.
const DONE: u64 = 0x5050_4d59;

/// log2 of each loop's iterations on the console: the probe's `ITER`.
const ITERATIONS_LOG2: u32 = 16;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 248);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[30], DONE, "the probe wrote every word");
    assert_eq!((words[28], words[29]), (5, u64::from(ITERATIONS_LOG2)));
    words
}

/// Loop `k`'s fastest run, in core cycles an iteration.
fn console_cycles(words: &[u64], k: usize) -> f64 {
    (words[2 * k] * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64
}

/// `std r14, 0(r29)`.
const STD: u32 = 0xf9dd_0000;
/// `stfd f14, 0(r29)`.
const STFD: u32 = 0xd9dd_0000;
/// `mfvscr v3`.
const MFVSCR: u32 = 0x1060_0604;
/// `li r10, 1`.
const LI_SAME: u32 = 0x3940_0001;
/// `li r3, 1` to `li r10, 1`.
const LI_DISTINCT: [u32; 8] = [
    0x3860_0001,
    0x3880_0001,
    0x38a0_0001,
    0x38c0_0001,
    0x38e0_0001,
    0x3900_0001,
    0x3920_0001,
    0x3940_0001,
];
/// `vaddubm v4, v1, v1` to `vaddubm v7, v1, v1`.
const VADD4: [u32; 4] = [0x1081_0800, 0x10a1_0800, 0x10c1_0800, 0x10e1_0800];

/// `body` closed by its `bdnz`.
fn looped(body: &[u32]) -> &'static [u32] {
    let mut words = body.to_vec();
    let back = (4 * words.len() as u32).wrapping_neg() & 0xfffc;
    words.push(0x4200_0000 | back);
    Box::leak(words.into_boxed_slice())
}

/// `n` `li r10, 1`.
fn same(n: usize) -> Vec<u32> {
    vec![LI_SAME; n]
}

/// `n` `li`, cycling r3 to r10.
fn distinct(n: usize) -> Vec<u32> {
    LI_DISTINCT.iter().copied().cycle().take(n).collect()
}

/// An `mfvscr`, then `rest`.
fn after_mfvscr(rest: Vec<u32>) -> Vec<u32> {
    let mut body = vec![MFVSCR];
    body.extend(rest);
    body
}

/// `head` and then `tail`.
fn then(mut head: Vec<u32>, tail: u32) -> Vec<u32> {
    head.push(tail);
    head
}

/// Each loop's body before its `bdnz`, in the probe's order.
fn bodies() -> [Vec<u32>; 14] {
    [
        same(32),
        distinct(32),
        after_mfvscr(distinct(8)),
        after_mfvscr(distinct(16)),
        after_mfvscr(distinct(32)),
        after_mfvscr(distinct(48)),
        after_mfvscr(distinct(64)),
        after_mfvscr(same(16)),
        after_mfvscr(same(48)),
        after_mfvscr(VADD4.to_vec()),
        then(after_mfvscr(distinct(16)), STFD),
        then(after_mfvscr(distinct(48)), STFD),
        then(after_mfvscr(distinct(48)), STD),
        then(after_mfvscr(distinct(1)), STD),
    ]
}

/// CellGov's cycles an iteration for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let r = Replay {
        loop_index: k,
        words: looped(&bodies()[k]),
        seeds: &[Seed::Gpr(29, ONE_LINE)],
    };
    replay(&r, 0).expect("the loop replays")
}

/// The share of a loop's fastest console run the model may lie on either
/// side of it.
const BAND: f64 = 0.06;

/// The loops the model holds:
///
/// - the runs of `li` alone;
/// - the runs of `li` to eight registers after an `mfvscr`;
/// - 16 `li` to one register after an `mfvscr`;
/// - one `li` and an `std` after an `mfvscr`.
const HELD: [usize; 9] = [0, 1, 2, 3, 4, 5, 6, 7, 13];

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

/// The loops the model does not hold, each shorter in the model than on
/// the console:
///
/// - 48 `li` to one register after an `mfvscr`;
/// - four `vaddubm` after an `mfvscr`;
/// - each run of `li` with a store after an `mfvscr`.
///
/// A second run of the probe took 71.13 and 46.85 cycles an iteration for
/// the first two loops, within the band of the model's 69 and 47. Loop 10
/// has the words of loop 3 of the `ppu_issue_store_drain_distance` probe
/// at a different address. That loop takes 67.00 cycles an iteration, and
/// the model holds it.
const DIFFERENT: [usize; 5] = [8, 9, 10, 11, 12];

/// The test fails when the model comes within the band of the console.
/// Then move the loop into `HELD`.
#[test]
fn each_other_loop_is_a_known_difference() {
    let w = console_words();
    for k in DIFFERENT {
        let (cellgov, console) = (cellgov_cycles(k), console_cycles(&w, k));
        assert!(
            cellgov < (1.0 - BAND) * console,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration now lies near the console's \
             {console:.2}; move the loop into HELD",
        );
    }
}
