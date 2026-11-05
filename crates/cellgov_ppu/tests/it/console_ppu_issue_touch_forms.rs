//! CellGov's PPU timing against the `ppu_issue_touch_forms` console
//! capture. The capture times:
//!
//! - the `dcbt` and `dcbtst` forms with a TH field;
//! - two touches a load;
//! - a touch of a line the L2 holds;
//! - `dcbst` or `dcbf` of a modified line.
//!
//! The tests do not assert loop 10, the store chase with no `dcbst` or
//! `dcbf`. The console takes 14 cycles a load, and CellGov takes 6.
//! CellGov does not delay a load of a line that an earlier store modified.

use crate::console_ppu_issue_timing::{replay, Replay, Seed};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_touch_forms/ps3/cech20-cex-493/observation.json";

/// The payload's word 25 when the probe wrote every word.
const DONE: u64 = 0x5050_544f;

/// log2 of each loop's iterations on the console.
const ITERATIONS_LOG2: u32 = 12;

/// The loads in each loop's body.
const LOADS: f64 = 4.0;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 208);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[25], DONE, "the probe wrote every word");
    assert_eq!((words[23], words[24]), (5, u64::from(ITERATIONS_LOG2)));
    words
}

/// Loop `k`'s fastest run, in core cycles a load.
fn console_cycles(words: &[u64], k: usize) -> f64 {
    (words[2 * k] * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64 / LOADS
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
/// `addis r11, r4, 64`: the line 4 MB ahead.
const FAR: u32 = 0x3d64_0040;
/// `dcbt 0, r10`.
const DCBT: u32 = 0x7c00_522c;
/// `dcbt 0, r11`.
const DCBT_FAR: u32 = 0x7c00_5a2c;
/// `dcbt 0, r10, 8`: TH = '1000', a data-stream setup form.
const DCBT_TH8: u32 = 0x7d00_522c;
/// `dcbt 0, r10, 10`: TH = '1010', a data-stream control form.
const DCBT_TH10: u32 = 0x7d40_522c;
/// `dcbtst 0, r10, 10`: TH = '1010'.
const DCBTST_TH10: u32 = 0x7d40_51ec;
/// `dcbt 0, r10, 1`: TH = '0001'.
const DCBT_TH1: u32 = 0x7c20_522c;
/// `std r4, 8(r4)`: modifies the line the next load reads.
const STD: u32 = 0xf884_0008;
/// `dcbst 0, r4`.
const DCBST: u32 = 0x7c00_206c;
/// `dcbf 0, r4`.
const DCBF: u32 = 0x7c00_20ac;

/// Where the replay places the 8 MB, 256 KB and 16 KB rings.
const MEM_RING: u64 = 0x100_0000;
const L2_RING: u64 = 0x40_0000;
const L1_RING: u64 = 0x80_0000;

const MEM_SEED: &[Seed] = &[Seed::Ring(4, MEM_RING, 8 * 1024 * 1024, false)];
const L2_SEED: &[Seed] = &[Seed::Ring(4, L2_RING, 256 * 1024, false)];
const L1_SEED: &[Seed] = &[Seed::Ring(4, L1_RING, 16 * 1024, false)];

/// Each loop's body and ring, in the probe's order.
fn loops() -> [(Vec<u32>, &'static [Seed]); 11] {
    let rep = |words: &[u32]| words.repeat(4);
    [
        (rep(&[AHEAD, DCBT_TH8, LD]), MEM_SEED),
        (rep(&[AHEAD, DCBT_TH10, LD]), MEM_SEED),
        (rep(&[AHEAD, DCBTST_TH10, LD]), MEM_SEED),
        (rep(&[AHEAD, DCBT_TH1, LD]), MEM_SEED),
        (rep(&[AHEAD, DCBT, FAR, DCBT_FAR, LD]), MEM_SEED),
        (rep(&[AHEAD, LD]), MEM_SEED),
        (rep(&[AHEAD, DCBT, LD]), L2_SEED),
        (rep(&[AHEAD, LD]), L2_SEED),
        (rep(&[STD, DCBST, LD]), L1_SEED),
        (rep(&[STD, DCBF, LD]), L1_SEED),
        (rep(&[STD, LD]), L1_SEED),
    ]
}

/// CellGov's cycles a load for loop `k`.
fn cellgov_cycles(k: usize, body: Vec<u32>, seeds: &'static [Seed]) -> f64 {
    let r = Replay {
        loop_index: k,
        words: looped(&body),
        seeds,
    };
    replay(&r, 0).expect("the loop replays") / LOADS
}

/// Each loop's cycles a load: (CellGov, the console's fastest run).
fn rows() -> Vec<(f64, f64)> {
    let w = console_words();
    loops()
        .into_iter()
        .enumerate()
        .map(|(k, (body, seeds))| (cellgov_cycles(k, body, seeds), console_cycles(&w, k)))
        .collect()
}

/// The largest share of a loop's fastest console run the model may differ
/// by.
const GAP: f64 = 0.02;

/// The gap of a chase in line order that no touch speeds up. The model's
/// load latencies come from other chases. A chase in line order runs
/// faster on the console, by up to 10% through memory.
const IN_ORDER_CHASE_GAP: f64 = 0.10;

/// The touch loops and the modified-line flush loops.
const TOUCHED: [usize; 6] = [2, 3, 4, 6, 8, 9];

/// The loops that touch no line: TH = '1000', TH = '1010', and the chase
/// alone through each ring.
const UNTOUCHED: [usize; 4] = [0, 1, 5, 7];

fn assert_within(rows: &[(f64, f64)], k: usize, gap: f64) {
    let (cellgov, console) = rows[k];
    assert!(
        (console - cellgov).abs() / console <= gap,
        "loop {k}: CellGov {cellgov:.2} cycles a load, the console's fastest {console:.2}, \
         more than {gap} of it apart: {rows:?}",
    );
}

#[test]
fn each_touch_form_and_modified_line_flush_times_within_two_percent_of_the_console() {
    let rows = rows();
    for k in TOUCHED {
        assert_within(&rows, k, GAP);
    }
}

#[test]
fn a_dcbt_stream_form_touches_no_line_and_times_as_the_chase_alone() {
    let rows = rows();
    for k in UNTOUCHED {
        assert_within(&rows, k, IN_ORDER_CHASE_GAP);
    }
    for k in [0, 1] {
        // A touch would fetch each line ahead of its load.
        for (who, cycles, alone) in [
            ("CellGov", rows[k].0, rows[5].0),
            ("console", rows[k].1, rows[5].1),
        ] {
            assert!(
                cycles >= alone,
                "loop {k}: {who} {cycles:.2} cycles a load, faster than the chase alone {alone:.2}",
            );
        }
    }
}
