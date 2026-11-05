//! CellGov's PPU timing against the `ppu_issue_vscr_fdiv` console
//! capture: what an `fdiv` or `fsqrt` costs when an `mtvscr` fence holds
//! it, and whether fixed-point instructions after it issue past it.
//!
//! The probe times ten loops with `mftb`, five runs each. Every loop
//! reads f20 = 1.0, f21 = 3.0 and f22 = 2.0, and every vector register it
//! reads starts zero, so `mtvscr v0` writes a zero VSCR.

use crate::console_ppu_issue_timing::{replay, Replay, Seed};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_vscr_fdiv/ps3/cech20-cex-493/observation.json";

/// The payload's word 22 when the probe wrote every word.
const DONE: u64 = 0x5050_5646;

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
/// `fdiv f1, f20, f21`.
const FDIV1: u32 = 0xfc34_a824;
/// `fdiv f2, f20, f21`.
const FDIV2: u32 = 0xfc54_a824;
/// `fsqrt f1, f22`.
const FSQRT: u32 = 0xfc20_b02c;
/// `fadd f3, f1, f1`.
const FADD: u32 = 0xfc61_082a;
/// `vaddubm v1, v20, v21`.
const VADD: u32 = 0x1034_a800;
/// `addi r14..r17, r14..r17, 1`.
const ADDI: [u32; 4] = [0x39ce_0001, 0x39ef_0001, 0x3a10_0001, 0x3a31_0001];

/// f20 = 1.0, f21 = 3.0 and f22 = 2.0.
const SEEDS: [Seed; 3] = [
    Seed::Fpr(20, 0x3ff0_0000_0000_0000),
    Seed::Fpr(21, 0x4008_0000_0000_0000),
    Seed::Fpr(22, 0x4000_0000_0000_0000),
];

/// Each loop's body, in the probe's order.
fn bodies() -> [Vec<u32>; 10] {
    let addi16 = ADDI.repeat(4);
    let with16 = |head: &[u32]| [head, &addi16[..]].concat();
    [
        vec![FDIV1],
        vec![MTVSCR, FDIV1],
        with16(&[MTVSCR, FDIV1]),
        vec![FDIV1, FDIV2],
        vec![MTVSCR, FDIV1, FDIV2],
        with16(&[MTVSCR, FDIV1, FDIV2]),
        vec![MTVSCR, FSQRT],
        with16(&[MTVSCR, FSQRT]),
        vec![MTVSCR, FDIV1, FADD],
        with16(&[MTVSCR, VADD, FDIV1]),
    ]
}

/// CellGov's cycles an iteration for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let r = Replay {
        loop_index: k,
        words: looped(&bodies()[k]),
        seeds: &SEEDS,
    };
    replay(&r, 0).expect("the loop replays")
}

/// The loops the model times above the console:
///
/// - an `mtvscr` and one `fdiv`, with and without 16 `addi`;
/// - an `mtvscr` and one `fsqrt`;
/// - an `mtvscr`, a `vaddubm`, an `fdiv` and 16 `addi`.
///
/// The console adds about 36 cycles to the `fdiv` and about 38 to the
/// `fsqrt` past the `mtvscr`. The model adds the `mtvscr`'s 42 cycles and
/// 14 more.
///
/// The `fsqrt` loop with 16 `addi` lies inside [`BAND`] only because
/// two differences cancel: on the console the `addi` add about 23 cycles
/// after the fenced `fsqrt`, and in the model none.
const FENCED_DIVIDE_DIFFERENCES: [usize; 4] = [1, 2, 6, 9];

/// The share of a loop's fastest console run the model may lie on either
/// side of it.
const BAND: f64 = 0.06;

#[test]
fn unfenced_divides_two_fenced_divides_and_a_fenced_divide_use_lie_within_the_band() {
    let w = console_words();
    let rows: Vec<(usize, f64, f64)> = (0..bodies().len())
        .filter(|k| !FENCED_DIVIDE_DIFFERENCES.contains(k))
        .map(|k| (k, cellgov_cycles(k), console_cycles(&w, k)))
        .collect();
    assert_eq!(rows.len(), bodies().len() - FENCED_DIVIDE_DIFFERENCES.len());
    for &(k, cellgov, console) in &rows {
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}, more than {BAND} of it apart: {rows:?}",
        );
    }
}

#[test]
fn one_fdiv_or_fsqrt_after_an_mtvscr_is_a_known_difference() {
    let w = console_words();
    for k in FENCED_DIVIDE_DIFFERENCES {
        let (cellgov, console) = (cellgov_cycles(k), console_cycles(&w, k));
        assert!(
            cellgov > (1.0 + BAND) * console,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration now lies near the console's \
             {console:.2}; move the loop into the held set",
        );
    }
}
