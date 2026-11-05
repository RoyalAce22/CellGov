//! CellGov's PPU timing against the `ppu_issue_mfvscr_lsu_waw` console
//! capture: whether the load or store part of a floating-point or vector
//! load or store younger than an `mfvscr` waits for the `mfvscr`'s result,
//! and how fast runs of fixed-point writes to one register issue.
//!
//! The probe times fourteen loops with `mftb`, five runs each. Every load
//! and store addresses one data line, which r29 holds.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_mfvscr_lsu_waw/ps3/cech20-cex-493/observation.json";

/// The payload's word 30 when the probe wrote every word.
const DONE: u64 = 0x5050_4d4c;

/// log2 of each loop's iterations on the console: the probe's `ITER`.
const ITERATIONS_LOG2: u32 = 16;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 248);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[30], DONE, "the probe wrote every word");
    assert_eq!((words[28], words[29]), (5, u64::from(ITERATIONS_LOG2)));
    words
}

/// Loop `k`'s fastest run, in core cycles an iteration.
fn console_cycles(words: &[u64], k: usize) -> f64 {
    (words[2 * k] * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64
}

/// `mfvscr v3`.
const MFVSCR: u32 = 0x1060_0604;
/// `li r3, 1` to `li r10, 1`.
const LI_DISTINCT: [u32; 8] = [
    0x3860_0001,
    0x3880_0001,
    0x38a0_0001,
    0x38c0_0001,
    0x38e0_0001,
    0x3900_0001,
    0x3920_0001,
    0x3940_0001,
];
/// `lwz r4, 0(r29)`.
const LWZ: u32 = 0x809d_0000;
/// `lfd f1, 0(r29)`.
const LFD: u32 = 0xc83d_0000;
/// `lvx v4, 0, r29`.
const LVX: u32 = 0x7c80_e8ce;
/// `stvx v1, 0, r29`.
const STVX: u32 = 0x7c20_e9ce;
/// `addi r29, r29, 0`.
const ADDI_BASE: u32 = 0x3bbd_0000;
/// `lfdu f1, 0(r29)`.
const LFDU: u32 = 0xcc3d_0000;
/// `stfdu f14, 0(r29)`.
const STFDU: u32 = 0xdddd_0000;
/// `addi r4, r29, 0`.
const ADDI_FROM_BASE: u32 = 0x389d_0000;
/// `addi r4, r4, 1`.
const ADDI_CHAIN: u32 = 0x3884_0001;
/// `add r10, r11, r11`.
const ADD: u32 = 0x7d4b_5a14;
/// `add r3, r11, r11` to `add r10, r11, r11`.
const ADD_DISTINCT: [u32; 8] = [
    0x7c6b_5a14,
    0x7c8b_5a14,
    0x7cab_5a14,
    0x7ccb_5a14,
    0x7ceb_5a14,
    0x7d0b_5a14,
    0x7d2b_5a14,
    0x7d4b_5a14,
];
/// `add. r10, r11, r11`.
const ADD_DOT: u32 = 0x7d4b_5a15;
/// `addic r10, r11, 1`.
const ADDIC: u32 = 0x314b_0001;
/// `rlwinm r10, r11, 1, 0, 31`.
const RLWINM: u32 = 0x556a_083e;
/// `srawi r10, r11, 1`.
const SRAWI: u32 = 0x7d6a_0e70;

/// `body` closed by its `bdnz`.
fn looped(body: &[u32]) -> &'static [u32] {
    let mut words = body.to_vec();
    let back = (4 * words.len() as u32).wrapping_neg() & 0xfffc;
    words.push(0x4200_0000 | back);
    Box::leak(words.into_boxed_slice())
}

/// An `mfvscr`, then `rest`.
fn after_mfvscr(rest: &[u32]) -> Vec<u32> {
    let mut body = vec![MFVSCR];
    body.extend_from_slice(rest);
    body
}

/// An `mfvscr`, `head`, then `addi r4, r29, 0` and seven `addi r4, r4, 1`.
fn chained(head: u32) -> Vec<u32> {
    let mut body = vec![MFVSCR, head, ADDI_FROM_BASE];
    body.extend([ADDI_CHAIN; 7]);
    body
}

/// `n` words of `pattern` repeated.
fn cycled(pattern: &[u32], n: usize) -> Vec<u32> {
    pattern.iter().copied().cycle().take(n).collect()
}

/// Each loop's body before its `bdnz`, in the probe's order.
fn bodies() -> [Vec<u32>; 14] {
    [
        after_mfvscr(&cycled(&LI_DISTINCT, 16)),
        after_mfvscr(&[LWZ]),
        after_mfvscr(&[LFD]),
        after_mfvscr(&[LVX]),
        after_mfvscr(&[STVX]),
        chained(ADDI_BASE),
        chained(LFDU),
        chained(STFDU),
        cycled(&[ADD], 32),
        cycled(&ADD_DISTINCT, 32),
        cycled(&[ADD_DOT], 32),
        cycled(&[ADDIC], 32),
        cycled(&[RLWINM], 32),
        cycled(&[SRAWI], 32),
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

const HELD: [usize; 14] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13];

#[test]
fn each_held_loop_lies_within_the_band() {
    let w = console_words();
    let misses: Vec<String> = HELD
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
