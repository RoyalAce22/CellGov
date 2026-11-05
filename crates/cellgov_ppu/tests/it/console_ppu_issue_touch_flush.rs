//! CellGov's PPU timing against the `ppu_issue_touch_flush` console
//! capture: what `dcbt`, `dcbtst` and `dcbf` do to a later load.
//!
//! The probe walks a ring of one doubleword per 128-byte line in line
//! order. Loops 0 to 3 walk 8 MB, so a load reaches memory unless a touch
//! fetched its line first. Loops 4 to 6 walk 16 KB, which the L1 holds, so
//! a load hits unless a flush removed its line.

use crate::console_ppu_issue_timing::{replay, Replay, Seed};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_touch_flush/ps3/cech20-cex-493/observation.json";

/// The payload's word 17 when the probe wrote every word.
const DONE: u64 = 0x5050_5446;

/// log2 of each loop's iterations on the console.
const ITERATIONS_LOG2: u32 = 12;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 144);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[17], DONE, "the probe wrote every word");
    assert_eq!((words[15], words[16]), (5, u64::from(ITERATIONS_LOG2)));
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

/// `ld r4, 0(r4)`.
const LD: u32 = 0xe884_0000;
/// `addi r10, r4, 1024`: the line eight ahead.
const AHEAD: u32 = 0x3944_0400;
/// `dcbt 0, r10`.
const DCBT: u32 = 0x7c00_522c;
/// `dcbtst 0, r10`.
const DCBTST: u32 = 0x7c00_51ec;
/// `dcbf 0, r4`.
const DCBF: u32 = 0x7c00_20ac;
/// `dcbst 0, r4`.
const DCBST: u32 = 0x7c00_206c;

/// Where the replay places the 8 MB ring and the 16 KB ring.
const MEM_RING: u64 = 0x100_0000;
const L1_RING: u64 = 0x80_0000;

const MEM_SEED: &[Seed] = &[Seed::Ring(4, MEM_RING, 8 * 1024 * 1024, false)];
const L1_SEED: &[Seed] = &[Seed::Ring(4, L1_RING, 16 * 1024, false)];

/// Each loop's body and ring, in the probe's order.
fn loops() -> [(Vec<u32>, &'static [Seed]); 7] {
    let rep = |words: &[u32], n: usize| words.repeat(n);
    [
        (rep(&[LD], 8), MEM_SEED),
        (rep(&[AHEAD, DCBT, LD], 4), MEM_SEED),
        (rep(&[AHEAD, DCBTST, LD], 4), MEM_SEED),
        (rep(&[AHEAD, LD], 4), MEM_SEED),
        (rep(&[DCBF, LD], 4), L1_SEED),
        (rep(&[DCBST, LD], 4), L1_SEED),
        (rep(&[LD], 8), L1_SEED),
    ]
}

/// The largest share of a loop's fastest console run the model may differ
/// by. `dcbtst` takes 285.0 cycles for four loads, and the model takes 280.
const GAP: f64 = 0.02;

/// The gap of loops 0 and 3, the chase through the 8 MB ring with no
/// touch. The model's memory latency comes from a chase over a shuffled
/// ring. A chase in line order takes 403 to 417 cycles a load on the
/// console, and 443 in the model.
const IN_ORDER_CHASE_GAP: f64 = 0.10;

#[test]
fn the_issue_model_times_each_touch_and_flush_loop_within_its_gap_of_the_console() {
    let w = console_words();
    let mut rows = Vec::new();
    for (k, (body, seeds)) in loops().into_iter().enumerate() {
        let r = Replay {
            loop_index: k,
            words: looped(&body),
            seeds,
        };
        let cellgov = replay(&r, 0).expect("the loop replays");
        let gap = if k == 0 || k == 3 {
            IN_ORDER_CHASE_GAP
        } else {
            GAP
        };
        rows.push((k, cellgov, console_cycles(&w, k), gap));
    }
    for &(k, cellgov, fastest, gap) in &rows {
        assert!(
            (fastest - cellgov).abs() / fastest <= gap,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {fastest:.2}, more than {gap} of it apart: {rows:?}",
        );
    }
}
