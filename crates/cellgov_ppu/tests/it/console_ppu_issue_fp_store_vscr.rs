//! CellGov's PPU timing against the `ppu_issue_fp_store_vscr` console
//! capture: whether each floating-point store form waits for an older
//! `mfvscr`'s result, and the wait with independent work between them.
//!
//! The probe times thirteen loops with `mftb`, five runs each. Every
//! store addresses one data line, which r29 holds.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_fp_store_vscr/ps3/cech20-cex-493/observation.json";

/// The payload's word 28 when the probe wrote every word.
const DONE: u64 = 0x5050_4656;

/// log2 of each loop's iterations on the console: the probe's `ITER`.
const ITERATIONS_LOG2: u32 = 16;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 232);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[28], DONE, "the probe wrote every word");
    assert_eq!((words[26], words[27]), (5, u64::from(ITERATIONS_LOG2)));
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
/// `stfs f14, 0(r29)`.
const STFS: u32 = 0xd1dd_0000;
/// `stfiwx f14, 0, r29`.
const STFIWX: u32 = 0x7dc0_efae;
/// `stfdx f14, 0, r29`.
const STFDX: u32 = 0x7dc0_edae;
/// `stfsx f14, 0, r29`.
const STFSX: u32 = 0x7dc0_ed2e;
/// `mfvscr v3`.
const MFVSCR: u32 = 0x1060_0604;
/// `li r10, 1`.
const LI: u32 = 0x3940_0001;

/// `body` closed by its `bdnz`.
fn looped(body: &[u32]) -> &'static [u32] {
    let mut words = body.to_vec();
    let back = (4 * words.len() as u32).wrapping_neg() & 0xfffc;
    words.push(0x4200_0000 | back);
    Box::leak(words.into_boxed_slice())
}

/// An `mfvscr`, `gap` `li`, then `store`.
fn spaced(gap: usize, store: u32) -> Vec<u32> {
    let mut body = vec![MFVSCR];
    body.extend(std::iter::repeat_n(LI, gap));
    body.push(store);
    body
}

/// Each loop's body before its `bdnz`, in the probe's order.
fn bodies() -> [Vec<u32>; 13] {
    [
        vec![MFVSCR],
        vec![STD, MFVSCR],
        vec![STFD, MFVSCR],
        vec![STFS, MFVSCR],
        vec![STFIWX, MFVSCR],
        vec![STFDX, MFVSCR],
        vec![STFSX, MFVSCR],
        spaced(8, STD),
        spaced(8, STFD),
        spaced(16, STD),
        spaced(16, STFD),
        spaced(32, STD),
        spaced(32, STFD),
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

fn assert_within_band(loops: impl IntoIterator<Item = usize>) {
    let w = console_words();
    let misses: Vec<String> = loops
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
fn each_fp_store_form_before_an_mfvscr_lies_within_the_band() {
    assert_within_band(0..7);
}

/// The loops with `li` between the `mfvscr` and the store that the model
/// holds: eight and 32 `li` before an `std`.
const SPACED_HELD: [usize; 2] = [7, 11];

#[test]
fn each_held_loop_with_li_before_its_store_lies_within_the_band() {
    assert_within_band(SPACED_HELD);
}

/// The loops with `li` between the `mfvscr` and the store that the model
/// does not hold. On the console an `stfd` costs about 19 cycles an
/// iteration more than an `std` at each distance, and 16 `li` before an
/// `std` take 48 cycles. The model gives 12 or 13 more for the `stfd`, and
/// 40 for the 16 `li`.
const SPACED_DIFFERENT: [usize; 4] = [8, 9, 10, 12];

/// The test fails when the model comes within the band of the console.
/// Then move the loop into `SPACED_HELD`.
#[test]
fn each_other_loop_with_li_before_its_store_is_a_known_difference() {
    let w = console_words();
    for k in SPACED_DIFFERENT {
        let (cellgov, console) = (cellgov_cycles(k), console_cycles(&w, k));
        assert!(
            cellgov < (1.0 - BAND) * console,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration now lies near the console's \
             {console:.2}; move the loop into SPACED_HELD",
        );
    }
}
