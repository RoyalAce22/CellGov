//! CellGov's PPU timing against the `ppu_issue_vscr_elem` console
//! capture: which VMX element stores and single-precision FP loads an
//! `mtvscr` holds past its fence, and for how long.
//!
//! The probe times twelve loops with `mftb`, five runs each. Every load
//! and store addresses one data line, which r8 holds.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_vscr_elem/ps3/cech20-cex-493/observation.json";

/// The payload's word 26 when the probe wrote every word.
const DONE: u64 = 0x5050_5645;

/// log2 of each loop's iterations on the console.
const ITERATIONS_LOG2: u32 = 18;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 216);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[26], DONE, "the probe wrote every word");
    assert_eq!((words[24], words[25]), (5, u64::from(ITERATIONS_LOG2)));
    words
}

/// Loop `k`'s fastest run, in core cycles an iteration.
fn console_cycles(words: &[u64], k: usize) -> f64 {
    (words[2 * k] * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64
}

/// `mtvscr v0`.
const MTVSCR: u32 = 0x1000_0644;
/// `stvebx v20, 0, r8`.
const STVEBX: u32 = 0x7e80_410e;
/// `stvebx v21, 0, r8`.
const STVEBX_V21: u32 = 0x7ea0_410e;
/// `stvehx v20, 0, r8`.
const STVEHX: u32 = 0x7e80_414e;
/// `stvewx v20, 0, r8`.
const STVEWX: u32 = 0x7e80_418e;
/// `stvx v21, 0, r8`.
const STVX: u32 = 0x7ea0_41ce;
/// `lfs f1, 0(r8)`.
const LFS: u32 = 0xc028_0000;
/// `lfs f3, 0(r8)`.
const LFS_F3: u32 = 0xc068_0000;
/// `lfsx f1, 0, r8`.
const LFSX: u32 = 0x7c20_442e;
/// `lfsu f1, 0(r8)`.
const LFSU: u32 = 0xc428_0000;
/// `lfd f1, 0(r8)`.
const LFD: u32 = 0xc828_0000;
/// `lvebx v1, 0, r8`.
const LVEBX: u32 = 0x7c20_400e;

/// Each loop's body after its `mtvscr`, in the probe's order.
const BODIES: [&[u32]; 12] = [
    &[STVEBX],
    &[STVEHX],
    &[STVEWX],
    &[STVX],
    &[LFS],
    &[LFSX],
    &[LFSU],
    &[LFD],
    &[STVEBX, STVEBX_V21],
    &[LFS, LFS_F3],
    &[LVEBX],
    &[],
];

/// `mtvscr` and loop `k`'s body, closed by its `bdnz`.
fn looped(k: usize) -> &'static [u32] {
    let mut words = vec![MTVSCR];
    words.extend_from_slice(BODIES[k]);
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

/// The largest share of a loop's fastest console run the model may fall
/// short by. The model holds every fenced VMX and FP access at the fence
/// and no longer. On the console, some loops take four to five cycles
/// more, and the set of these loops changes from capture to capture.
/// `stvebx` takes 63.18 cycles in `ppu_issue_vscr_mem_forms` and 59.61
/// here. `stvx` takes 59.55 in `ppu_issue_vscr_mem` and 63.23 here. Here
/// `lfs` takes 63.69, which is 8.9% over the model's 58.
pub(crate) const SPREAD: f64 = 0.09;

#[test]
fn an_mtvscr_holds_each_element_store_and_single_precision_load_at_the_fence_within_the_spread() {
    let w = console_words();
    let rows: Vec<(usize, f64, f64)> = (0..BODIES.len())
        .map(|k| (k, cellgov_cycles(k), console_cycles(&w, k)))
        .collect();
    assert_eq!(rows.len(), 12);
    for &(k, cellgov, fastest) in &rows {
        assert!(
            cellgov <= fastest && (fastest - cellgov) / fastest <= SPREAD,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {fastest:.2}, more than {SPREAD} of it apart or above it: {rows:?}",
        );
    }
}
