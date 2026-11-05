//! CellGov's PPU timing against the `ppu_issue_store_drain_distance`
//! console capture: how long a drain after a store waits for it, by the
//! store's form and by the distance between them.
//!
//! The probe times twenty-six loops with `mftb`, five runs each. Every
//! store addresses one data line, which r29 holds.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_store_drain_distance/ps3/cech20-cex-493/observation.json";

/// The loops the probe times.
const LOOPS: usize = 26;

/// The payload's word 54 when the probe wrote every word.
const DONE: u64 = 0x5050_5344;

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

/// `std r14, 0(r29)`.
const STD: u32 = 0xf9dd_0000;
/// `std r14, 8(r29)`.
const STD_8: u32 = 0xf9dd_0008;
/// `stw r14, 0(r29)`.
const STW: u32 = 0x91dd_0000;
/// `stb r14, 0(r29)`.
const STB: u32 = 0x99dd_0000;
/// `stfd f14, 0(r29)`.
const STFD: u32 = 0xd9dd_0000;
/// `stfs f14, 0(r29)`.
const STFS: u32 = 0xd1dd_0000;
/// `stfiwx f14, 0, r29`.
const STFIWX: u32 = 0x7dc0_efae;
/// `stvx v1, 0, r29`.
const STVX: u32 = 0x7c20_e9ce;
/// `mfvscr v3`.
const MFVSCR: u32 = 0x1060_0604;
/// `mffs f0`.
const MFFS: u32 = 0xfc00_048e;
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

fn body(parts: &[&[u32]]) -> Vec<u32> {
    parts.concat()
}

/// Each loop's body before its `bdnz`, in the probe's order.
fn bodies() -> [Vec<u32>; LOOPS] {
    let (li4, li8, li16, li32, li48) = (li(4), li(8), li(16), li(32), li(48));
    let held = |rest: &[u32]| body(&[&[MFVSCR], &li16, rest]);
    [
        held(&[STD]),
        held(&[STW]),
        held(&[STB]),
        held(&[STFD]),
        held(&[STFS]),
        held(&[STFIWX]),
        held(&[STVX]),
        held(&[STD, STD_8]),
        held(&body(&[&[STD], &li4])),
        held(&body(&[&[STD], &li8])),
        held(&body(&[&[STD], &li16])),
        held(&body(&[&[STD], &li32])),
        held(&body(&[&[STFD], &li8])),
        held(&body(&[&[STFD], &li16])),
        held(&body(&[&[STFD], &li32])),
        held(&body(&[&[STFD], &li48])),
        vec![MFFS],
        body(&[&li16, &[MFFS]]),
        body(&[&li16, &[STD, MFFS]]),
        body(&[&li16, &[STFD, MFFS]]),
        body(&[&li32, &[MFFS]]),
        body(&[&li32, &[STD, MFFS]]),
        body(&[&li32, &[STFD, MFFS]]),
        body(&[&[STD], &li8, &[MFFS]]),
        held(&[]),
        body(&[&[MFVSCR], &li32]),
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
/// - an `mfvscr` after each store form, at the end of the held work or
///   with `li` between them;
/// - an `mfvscr` after two `std`;
/// - an `mffs` after a run of `li`, alone or with an `std` before it;
/// - the runs of `li` after an `mfvscr` with no store.
const HELD: [usize; 23] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 21, 23, 24, 25,
];

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
fn an_mfvscr_waits_for_an_older_store_by_its_register_file() {
    // The fixed-point, vector and floating-point stores at the end of the
    // held work each cost the next `mfvscr` more, in that order, on the
    // console and in the model.
    let w = console_words();
    let order = |cycles: &dyn Fn(usize) -> f64| {
        let (fixed, vector, float) = (cycles(0), cycles(6), cycles(3));
        let none = cycles(24);
        none < fixed && fixed < vector && vector < float
    };
    assert!(order(&|k| console_cycles(&w, k)), "the console");
    assert!(order(&cellgov_cycles), "CellGov");
}

/// The loops the model does not hold, each with the side of the console
/// the model lies on:
///
/// - an `mffs` after a run of `li`, or after a run of `li` and an `stfd`.
///   [CBE-Handbook p:722 s:A.1.2] `mffs` stalls at VQ8 until older VSU
///   operations complete; the model stalls it at issue.
const DIFFERENT: [(usize, Side); 3] = [(19, Side::Under), (20, Side::Over), (22, Side::Over)];

/// The side of the console's fastest run a known difference lies on.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Side {
    Under,
    Over,
}

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
