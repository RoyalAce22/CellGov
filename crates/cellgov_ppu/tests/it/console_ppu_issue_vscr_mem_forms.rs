//! CellGov's PPU timing against the `ppu_issue_vscr_mem_forms` console
//! capture: whether an `mtvscr` stalls the VMX and FP load and store
//! forms `ppu_issue_vscr_mem` does not time.
//!
//! The probe times sixteen loops with `mftb`, five runs each. Every load
//! and store addresses one data line, which r8 holds.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_vscr_mem_forms/ps3/cech20-cex-493/observation.json";

/// The payload's word 34 when the probe wrote every word.
const DONE: u64 = 0x5050_564d;

/// log2 of each loop's iterations on the console.
const ITERATIONS_LOG2: u32 = 18;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 280);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[34], DONE, "the probe wrote every word");
    assert_eq!((words[32], words[33]), (5, u64::from(ITERATIONS_LOG2)));
    words
}

/// Loop `k`'s fastest run, in core cycles an iteration.
fn console_cycles(words: &[u64], k: usize) -> f64 {
    (words[2 * k] * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64
}

/// `mtvscr v0`.
const MTVSCR: u32 = 0x1000_0644;

/// The access each loop pair times, in the probe's order: `lvsl v1,0,r8`,
/// `lvebx v1,0,r8`, `lvlx v1,0,r8`, `stvebx v20,0,r8`, `lfs f1,0(r8)`,
/// `stfs f2,0(r8)`, `lfdu f1,0(r8)` and `stfiwx f2,0,r8`.
const ACCESSES: [u32; 8] = [
    0x7c20_400c,
    0x7c20_400e,
    0x7c20_440e,
    0x7e80_410e,
    0xc028_0000,
    0xd048_0000,
    0xcc28_0000,
    0x7c40_47ae,
];

/// Loop `k`'s body closed by its `bdnz`: the access alone for an even
/// `k`, after an `mtvscr` for an odd one.
fn looped(k: usize) -> &'static [u32] {
    let access = ACCESSES[k / 2];
    let mut words = if k.is_multiple_of(2) {
        vec![access]
    } else {
        vec![MTVSCR, access]
    };
    words.push(0x4200_0000 | ((4 * words.len() as u32).wrapping_neg() & 0xfffc));
    Box::leak(words.into_boxed_slice())
}

/// CellGov's cycles an iteration for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let r = Replay {
        loop_index: k,
        words: looped(k),
        seeds: &[Seed::Gpr(8, ONE_LINE)],
    };
    replay(&r, 0).expect("the loop replays")
}

/// The loops of an `mtvscr` and then an `lvsl`, an `lvebx`, an `lvlx`, an
/// `stfs`, an `lfdu` or an `stfiwx`.
const FENCED_HELD: [usize; 6] = [1, 3, 5, 11, 13, 15];

/// The loops of an `mtvscr` and then an `stvebx` or an `lfs`. In this
/// capture they take about five cycles more than the other fenced loops.
/// The `ppu_issue_vscr_elem` capture shows the same excess on other
/// forms, so the test checks these two against that capture's spread.
const FENCED_SPREAD: [usize; 2] = [7, 9];

/// The largest share of a fenced loop's fastest console run the model may
/// fall short by: the `lvlx` loop falls 3.8% short.
const GAP: f64 = 0.04;

#[test]
fn an_mtvscr_holds_lvsl_lvebx_lvlx_lfdu_stfs_and_stfiwx_after_it_within_the_gap() {
    let w = console_words();
    let rows: Vec<(usize, f64, f64)> = FENCED_HELD
        .iter()
        .map(|&k| (k, cellgov_cycles(k), console_cycles(&w, k)))
        .collect();
    for &(k, cellgov, fastest) in &rows {
        assert!(
            cellgov <= fastest && (fastest - cellgov) / fastest <= GAP,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {fastest:.2}, more than {GAP} of it apart or above it: {rows:?}",
        );
    }
}

#[test]
fn an_mtvscr_holds_stvebx_and_lfs_after_it_within_the_spread() {
    let spread = crate::console_ppu_issue_vscr_elem::SPREAD;
    let w = console_words();
    let rows: Vec<(usize, f64, f64)> = FENCED_SPREAD
        .iter()
        .map(|&k| (k, cellgov_cycles(k), console_cycles(&w, k)))
        .collect();
    for &(k, cellgov, fastest) in &rows {
        assert!(
            cellgov <= fastest && (fastest - cellgov) / fastest <= spread,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {fastest:.2}, more than {spread} of it apart or above it: {rows:?}",
        );
    }
}
