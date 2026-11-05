//! CellGov's PPU timing against the `ppu_issue_vscr_queue_depth` console
//! capture:
//!
//! - how many vector instructions an `mtvscr` fence holds before IS2
//!   stalls the fixed-point work after them;
//! - whether a vector or FP load that the fence holds takes the
//!   load-hit-store recycle of a store before it.
//!
//! The probe times ten loops with `mftb`, five runs each. Every vector
//! register starts zero, so `mtvscr v0` writes a zero VSCR. Every load
//! and store addresses one data line, which r8 holds.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_vscr_queue_depth/ps3/cech20-cex-493/observation.json";

/// The payload's word 22 when the probe wrote every word.
const DONE: u64 = 0x5050_5644;

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
/// `stw r14, 0(r8)`.
const STW: u32 = 0x91c8_0000;
/// `lfd f1, 0(r8)`.
const LFD: u32 = 0xc828_0000;
/// `lvx v1, 0, r8`.
const LVX: u32 = 0x7c20_40ce;
/// `vaddubm v2, v1, v1`.
const VADD_USE: u32 = 0x1041_0800;
/// `addi r15..r18, r15..r18, 1`.
const ADDI: [u32; 4] = [0x39ef_0001, 0x3a10_0001, 0x3a31_0001, 0x3a52_0001];

/// `n` independent `vaddubm`: v1 to v19, then v22 on, past the sources.
fn vadds(n: u32) -> Vec<u32> {
    (1..=19).chain(22..=26).take(n as usize).map(vadd).collect()
}

/// Each loop's body, in the probe's order.
fn bodies() -> [Vec<u32>; 10] {
    let addi16 = ADDI.repeat(4);
    let fenced = |v: u32, tail: &[u32]| [vec![MTVSCR], vadds(v), tail.to_vec()].concat();
    [
        fenced(8, &addi16),
        fenced(10, &addi16),
        fenced(12, &addi16),
        fenced(16, &addi16),
        fenced(24, &addi16),
        fenced(24, &[]),
        vec![MTVSCR, STW, LFD],
        vec![STW, LFD],
        vec![MTVSCR, LFD],
        [&[MTVSCR, LVX][..], &ADDI, &[VADD_USE]].concat(),
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

/// The loops whose `lfd` hits the `stw` before it. The model times the
/// load-hit-store recycle of an FP load as that of a fixed-point load:
/// the plain loop takes 31 cycles an iteration in the model and 66 on
/// the console.
const LOAD_HIT_STORE: [usize; 2] = [6, 7];

/// The share of a loop's fastest console run the model may lie on either
/// side of it. The model holds an `mtvscr` 42 cycles, from the
/// `ppu_issue_vscr` capture's 42.5. This capture's fenced loops take two
/// cycles less, up to 3% of them.
const BAND: f64 = 0.06;

#[test]
fn an_mtvscr_fence_holds_24_vector_instructions_and_the_fixed_point_work_issues_past_them() {
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
fn an_fp_load_that_hits_a_fixed_point_store_is_a_known_difference() {
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
