//! CellGov's PPU timing against the `ppu_issue_store_gather_hit` console
//! capture.
//!
//! The probe times, with `mftb`, eleven runs each:
//! - one `stw` to line A, then a load of its word, of the other word of
//!   its doubleword, of the other doubleword of its block, of the next
//!   block, or of a line no store writes;
//! - two `stw` to one 16-byte block, then a load of bytes in the block
//!   that neither wrote, and the same with the second store in another
//!   line;
//! - an `stw`, 96 `addi`, a second `stw` to the same block or to another
//!   line, then a load of the first store's word; and the 96 `addi` alone.
//!
//! The replay holds each loop's cycles an iteration no farther from the
//! fastest run than the slowest run is.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_store_gather_hit/ps3/cech20-cex-493/observation.json";

/// The loops the probe times.
const LOOP_COUNT: usize = 12;

/// log2 of each loop's iterations on the console, from the probe's
/// header.
const ITERATIONS_LOG2: u32 = 16;

/// The payload's word 26 when the probe wrote every word.
const DONE: u64 = 0x5053_4748;

/// The runs the probe takes of each loop.
const REPS: u64 = 11;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 8 * (2 * LOOP_COUNT + 3));
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(
        words[2 * LOOP_COUNT + 2],
        DONE,
        "the probe wrote every word"
    );
    assert_eq!(words[2 * LOOP_COUNT + 1], REPS, "the runs a loop");
    words
}

/// Loop `k`'s fastest and slowest runs, in core cycles an iteration.
fn console_cycles(words: &[u64], k: usize) -> (f64, f64) {
    let per = |w: u64| (w * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64;
    (per(words[2 * k]), per(words[2 * k + 1]))
}

/// `stw r9,0(r7)`: the store of the chained word to line A.
const STW_A: u32 = 0x9127_0000;
/// `stw r10,d(r7)`.
const STW_R10: u32 = 0x9147_0000;
/// `lwz r9,d(r7)`.
const LWZ: u32 = 0x8127_0000;
/// `addi r16,r16,1`, `addi r17,r17,1` and `addi r18,r18,1`.
const ADDI: [u32; 3] = [0x3a10_0001, 0x3a31_0001, 0x3a52_0001];
/// The `addi` between the two stores of loops 9 and 10.
const PAD: usize = 96;
/// The offset of line H, which no store writes.
const LINE_H: u32 = 1408;
/// The offset of line B.
const LINE_B: u32 = 128;

/// A loop body: the `stw` to line A, `pad` `addi`, a second `stw` at
/// `second` when it is `Some`, the `lwz` at `load`, and the `bdnz` back
/// to the first word.
const fn body<const N: usize>(pad: usize, second: Option<u32>, load: u32) -> [u32; N] {
    let mut words = [0; N];
    words[0] = STW_A;
    let mut i = 1;
    while i <= pad {
        words[i] = ADDI[(i - 1) % 3];
        i += 1;
    }
    if let Some(at) = second {
        words[i] = STW_R10 | at;
        i += 1;
    }
    words[i] = LWZ | load;
    // bdnz with a word displacement back to the loop's first word.
    words[N - 1] = 0x4200_0000 | (0x1_0000 - 4 * (N as u32 - 1));
    words
}

/// The 96 `addi` alone, and the `bdnz`.
const fn addi_only<const N: usize>() -> [u32; N] {
    let mut words = [0; N];
    let mut i = 0;
    while i < N - 1 {
        words[i] = ADDI[i % 3];
        i += 1;
    }
    words[N - 1] = 0x4200_0000 | (0x1_0000 - 4 * (N as u32 - 1));
    words
}

const LOOP_0: [u32; 3] = body(0, None, 0);
const LOOP_1: [u32; 3] = body(0, None, 4);
const LOOP_2: [u32; 3] = body(0, None, 8);
const LOOP_3: [u32; 3] = body(0, None, 16);
const LOOP_4: [u32; 3] = body(0, None, LINE_H);
const LOOP_5: [u32; 4] = body(0, Some(8), 4);
const LOOP_6: [u32; 4] = body(0, Some(12), 8);
const LOOP_7: [u32; 4] = body(0, Some(8), 12);
const LOOP_8: [u32; 4] = body(0, Some(LINE_B), 4);
const LOOP_9: [u32; PAD + 4] = body(PAD, Some(4), 0);
const LOOP_10: [u32; PAD + 4] = body(PAD, Some(LINE_B), 0);
const LOOP_11: [u32; PAD + 1] = addi_only();

/// Each loop's body as the built probe lays it out, with r7 line A.
const LOOPS: [&[u32]; LOOP_COUNT] = [
    &LOOP_0, &LOOP_1, &LOOP_2, &LOOP_3, &LOOP_4, &LOOP_5, &LOOP_6, &LOOP_7, &LOOP_8, &LOOP_9,
    &LOOP_10, &LOOP_11,
];

/// Each loop replayed alone, in cycles an iteration.
fn cellgov_cycles() -> Vec<f64> {
    let seeds: &'static [Seed] = &[Seed::Gpr(7, ONE_LINE)];
    LOOPS
        .iter()
        .enumerate()
        .map(|(k, words)| {
            let r = Replay {
                loop_index: k,
                words,
                seeds,
            };
            replay(&r, 0).expect("the loop replays")
        })
        .collect()
}

#[test]
fn the_issue_model_times_each_store_gather_hit_loop_within_the_console_s_spread() {
    let w = console_words();
    let cycles = cellgov_cycles();
    assert_eq!(cycles.len(), LOOPS.len());
    let rows: Vec<_> = cycles
        .iter()
        .enumerate()
        .map(|(k, &cellgov)| {
            let (fastest, slowest) = console_cycles(&w, k);
            (k, cellgov, fastest, slowest - fastest)
        })
        .collect();
    for &(k, cellgov, fastest, spread) in &rows {
        assert!(
            (cellgov - fastest).abs() <= spread,
            "loop {k}: CellGov {cellgov:.3} cycles an iteration, the console's \
             fastest {fastest:.3} with a spread of {spread:.3}: {rows:?}",
        );
    }
}

#[test]
fn a_load_of_bytes_no_pending_store_wrote_waits_for_no_store() {
    let w = console_words();
    let cycles = cellgov_cycles();
    // Loop 4 loads a line no store writes; loops 1 to 3 and 5 to 8 load
    // bytes in or beside a stored block that no store wrote.
    let (none, _) = console_cycles(&w, 4);
    let (hit, _) = console_cycles(&w, 0);
    assert!(
        hit > none + 30.0,
        "the console's load of the stored word waits: {hit:.3}, {none:.3} with no store",
    );
    for k in [1, 2, 3, 5, 6, 7, 8] {
        let (fastest, _) = console_cycles(&w, k);
        assert!(
            (fastest - none).abs() < 1.0,
            "loop {k}: the console waits for no store: {fastest:.3}, {none:.3} with no store",
        );
        assert!(
            (cycles[k] - cycles[4]).abs() < 1.0,
            "loop {k}: a load of bytes no store wrote waits for a store: CellGov \
             {:.3}, {:.3} with no store; the console {fastest:.3}",
            cycles[k],
            cycles[4],
        );
    }
}

#[test]
fn a_store_to_the_block_of_a_written_store_holds_no_load_of_the_written_bytes() {
    let w = console_words();
    let cycles = cellgov_cycles();
    // Loop 9 puts its second store in the first store's block, loop 10 in
    // line B; both load the first store's word after that store writes its
    // bytes into the cache.
    let (same, _) = console_cycles(&w, 9);
    let (apart, _) = console_cycles(&w, 10);
    assert!(
        same <= apart,
        "the console's load waits for no later store: {same:.3} with the second \
         store in the block, {apart:.3} in line B",
    );
    assert!(
        cycles[9] <= cycles[10] + 1.0,
        "the load of a written store's bytes waits for a later store to its block: \
         CellGov {:.3}, {:.3} with that store in line B; the console {same:.3} and \
         {apart:.3}",
        cycles[9],
        cycles[10],
    );
}
