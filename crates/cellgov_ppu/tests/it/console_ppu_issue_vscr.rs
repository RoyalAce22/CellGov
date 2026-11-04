//! CellGov's PPU timing against the `ppu_issue_vscr` console capture:
//! what `mfvscr` and `mtvscr` cost at issue.
//!
//! The probe times eight loops with `mftb`, five runs each. Every vector
//! register starts zero, so `mtvscr v0` writes a zero VSCR.

use crate::console_ppu_issue_timing::{replay, Replay};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_vscr/ps3/cech20-cex-493/observation.json";

/// The payload's word 18 when the probe wrote every word.
const DONE: u64 = 0x5050_5653;

/// log2 of each loop's iterations on the console.
const ITERATIONS_LOG2: u32 = 18;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 152);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[18], DONE, "the probe wrote every word");
    assert_eq!((words[16], words[17]), (5, u64::from(ITERATIONS_LOG2)));
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

/// `vaddubm vk, v20, v21`.
const fn vadd(k: u32) -> u32 {
    0x1014_a800 | (k << 21)
}

/// `mfvscr v1`.
const MFVSCR: u32 = 0x1020_0604;
/// `vaddubm v2, v1, v1`.
const VADD_USE: u32 = 0x1041_0800;
/// `mtvscr v0`.
const MTVSCR: u32 = 0x1000_0644;
/// `addi r14..r17, r14..r17, 1`.
const ADDI: [u32; 4] = [0x39ce_0001, 0x39ef_0001, 0x3a10_0001, 0x3a31_0001];

/// Each loop's body, in the probe's order.
fn bodies() -> [Vec<u32>; 8] {
    let vadd4: Vec<u32> = (1..=4).map(vadd).collect();
    let vadd8: Vec<u32> = (1..=8).map(vadd).collect();
    [
        vadd8,
        vec![MFVSCR],
        vec![MFVSCR, VADD_USE],
        vec![MTVSCR],
        [&[MTVSCR][..], &vadd4].concat(),
        [&[MTVSCR][..], &ADDI].concat(),
        [&vadd4[..], &[MTVSCR]].concat(),
        vadd4,
    ]
}

/// CellGov's cycles an iteration for loop `k`.
fn cellgov_cycles(k: usize, body: &[u32]) -> f64 {
    let r = Replay {
        loop_index: k,
        words: looped(body),
        seeds: &[],
    };
    replay(&r, 0).expect("the loop replays")
}

/// The largest share of a loop's fastest console run the model may fall
/// short by: the baseline loop 7, four `vaddubm` with no VSCR access,
/// falls 3.4% short.
const GAP: f64 = 0.04;

/// Loop 6's gap: four `vaddubm` before the `mtvscr` take 67.29 cycles an
/// iteration on the console and 63 in the model, which starts the next
/// iteration's `vaddubm` at the fence the `mtvscr` sets.
const LOOP_6_GAP: f64 = 0.07;

#[test]
fn the_issue_model_times_each_vscr_loop_at_most_its_gap_below_the_console() {
    let w = console_words();
    let mut rows = Vec::new();
    for (k, body) in bodies().iter().enumerate() {
        let gap = if k == 6 { LOOP_6_GAP } else { GAP };
        rows.push((k, cellgov_cycles(k, body), console_cycles(&w, k), gap));
    }
    for &(k, cellgov, fastest, gap) in &rows {
        assert!(
            cellgov <= fastest && (fastest - cellgov) / fastest <= gap,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {fastest:.2}, more than {gap} of it apart or above it: {rows:?}",
        );
    }
}
