//! CellGov's PPU timing against the `ppu_issue_vscr_lhs` console capture:
//! what a load that hits a store in the store queue costs by load and
//! store form, and how an `mtvscr` before the store changes it.
//!
//! The probe times ten loops with `mftb`, five runs each. Every load and
//! store addresses the first bytes of one data line, which r8 holds.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_vscr_lhs/ps3/cech20-cex-493/observation.json";

/// The payload's word 22 when the probe wrote every word.
const DONE: u64 = 0x5050_564c;

/// log2 of each loop's iterations on the console.
const ITERATIONS_LOG2: u32 = 16;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 184);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[22], DONE, "the probe wrote every word");
    assert_eq!((words[20], words[21]), (5, u64::from(ITERATIONS_LOG2)));
    words
}

/// Loop `k`'s fastest run, in core cycles an iteration.
fn console_cycles(words: &[u64], k: usize) -> f64 {
    (words[2 * k] * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64
}

/// `body` closed by its `bdnz`.
fn looped(body: &[u32]) -> &'static [u32] {
    let mut words = body.to_vec();
    words.push(0x4200_0000 | ((4 * body.len() as u32).wrapping_neg() & 0xfffc));
    Box::leak(words.into_boxed_slice())
}

/// `mtvscr v0`.
const MTVSCR: u32 = 0x1000_0644;
/// `stw r14, 0(r8)`.
const STW: u32 = 0x91c8_0000;
/// `std r14, 0(r8)`.
const STD: u32 = 0xf9c8_0000;
/// `stfd f2, 0(r8)`.
const STFD: u32 = 0xd848_0000;
/// `lwz r19, 0(r8)`.
const LWZ: u32 = 0x8268_0000;
/// `lfd f1, 0(r8)`.
const LFD: u32 = 0xc828_0000;
/// `lfs f1, 0(r8)`.
const LFS: u32 = 0xc028_0000;

/// Each loop's body, in the probe's order.
fn bodies() -> [Vec<u32>; 10] {
    [
        vec![STW, LWZ],
        vec![STD, LFD],
        vec![STFD, LFD],
        vec![STW, LFS],
        vec![STW, LFD],
        vec![MTVSCR, STW, LWZ],
        vec![MTVSCR, STD, LFD],
        vec![MTVSCR, STFD, LFD],
        vec![MTVSCR, LWZ],
        vec![MTVSCR, STW],
    ]
}

/// CellGov's cycles an iteration for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let r = Replay {
        loop_index: k,
        words: looped(&bodies()[k]),
        seeds: &[Seed::Gpr(8, ONE_LINE)],
    };
    replay(&r, 0).expect("the loop replays")
}

/// The loops whose load hits the store before it. The model times every
/// form's recycle at 31 cycles an iteration. On the console:
///
/// - an `lwz` after an `stw` takes 39;
/// - an `lfd` or `lfs` after any store takes 64 to 66;
/// - an `mtvscr` before an `stw` or `std` adds about its own 40 cycles,
///   for the fixed-point load as well, where the model overlaps the two;
/// - an `mtvscr` before an `stfd` adds about 74 cycles, 140 in all.
const LOAD_HIT_STORE: [usize; 8] = [0, 1, 2, 3, 4, 5, 6, 7];

/// The share of a loop's fastest console run the model may lie on either
/// side of it.
const BAND: f64 = 0.06;

#[test]
fn an_mtvscr_with_one_fixed_point_load_or_store_after_it_lies_within_the_band() {
    let w = console_words();
    let rows: Vec<(usize, f64, f64)> = (0..bodies().len())
        .filter(|k| !LOAD_HIT_STORE.contains(k))
        .map(|k| (k, cellgov_cycles(k), console_cycles(&w, k)))
        .collect();
    assert_eq!(rows.len(), bodies().len() - LOAD_HIT_STORE.len());
    for &(k, cellgov, console) in &rows {
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}, more than {BAND} of it apart: {rows:?}",
        );
    }
}

#[test]
fn each_load_that_hits_a_queued_store_is_a_known_difference() {
    let w = console_words();
    for k in LOAD_HIT_STORE {
        let (cellgov, console) = (cellgov_cycles(k), console_cycles(&w, k));
        assert!(
            cellgov < (1.0 - BAND) * console,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration now lies near the console's \
             {console:.2}; move the loop into the held set",
        );
    }
}
