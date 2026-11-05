//! CellGov's PPU timing against the `ppu_issue_vscr_queue` console
//! capture: whether fixed-point instructions after an `mtvscr` and a
//! vector instruction issue past the vector instruction the fence holds.
//!
//! The probe times ten loops with `mftb`, five runs each. Every vector
//! register starts zero, so `mtvscr v0` writes a zero VSCR.

use crate::console_ppu_issue_timing::{replay, Replay};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_vscr_queue/ps3/cech20-cex-493/observation.json";

/// The payload's word 22 when the probe wrote every word.
const DONE: u64 = 0x5050_5651;

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

/// `vaddubm vk, v20, v21`.
const fn vadd(k: u32) -> u32 {
    0x1014_a800 | (k << 21)
}

/// `mtvscr v0`.
const MTVSCR: u32 = 0x1000_0644;

/// `n` independent `addi`, cycling r14 to r17.
fn addi(n: u32) -> Vec<u32> {
    (0..n)
        .map(|i| {
            let r = 14 + i % 4;
            0x3800_0001 | (r << 21) | (r << 16)
        })
        .collect()
}

/// `n` independent `vaddubm`, v1 to vn.
fn vadds(n: u32) -> Vec<u32> {
    (1..=n).map(vadd).collect()
}

/// Each loop's body, in the probe's order.
fn bodies() -> [Vec<u32>; 10] {
    let fenced = |v: u32, a: u32| [vec![MTVSCR], vadds(v), addi(a)].concat();
    [
        vec![MTVSCR],
        fenced(1, 0),
        fenced(1, 4),
        fenced(1, 8),
        fenced(1, 16),
        fenced(1, 32),
        fenced(4, 16),
        fenced(8, 16),
        fenced(0, 16),
        addi(32),
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
/// side of it. The model holds an `mtvscr` 42 cycles, from the
/// `ppu_issue_vscr` capture's 42.5. This capture's `mtvscr` loop takes 40,
/// so every `mtvscr` loop here lies two cycles, up to 5%, above.
const BAND: f64 = 0.06;

#[test]
fn fixed_point_work_issues_past_a_vector_instruction_the_fence_holds_within_the_band() {
    let w = console_words();
    let rows: Vec<(usize, f64, f64)> = (0..bodies().len())
        .map(|k| (k, cellgov_cycles(k), console_cycles(&w, k)))
        .collect();
    for &(k, cellgov, console) in &rows {
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}, more than {BAND} of it apart: {rows:?}",
        );
    }
}
