//! CellGov's PPU timing against the `ppu_issue_mfxer` console capture:
//! what an `mfxer` costs at issue, alone and beside fixed-point work.
//!
//! The probe times ten loops with `mftb`, five runs each.

use crate::console_ppu_issue_timing::{replay, Replay, Seed};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_mfxer/ps3/cech20-cex-493/observation.json";

/// The payload's word 22 when the probe wrote every word.
const DONE: u64 = 0x5050_5852;

/// log2 of each loop's iterations on the console: the probe's `ITER`.
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

/// `mfxer rk`.
const fn mfxer(k: u32) -> u32 {
    0x7c01_02a6 | (k << 21)
}
/// `mtxer rk`.
const fn mtxer(k: u32) -> u32 {
    0x7c01_03a6 | (k << 21)
}
/// `mfcr rk`.
const fn mfcr(k: u32) -> u32 {
    0x7c00_0026 | (k << 21)
}
/// `li rk, 1`.
const fn li(k: u32) -> u32 {
    0x3800_0001 | (k << 21)
}
/// `addi r4, r3, 1`.
const ADDI_USE: u32 = 0x3883_0001;
/// `addic r4, r4, 1`.
const ADDIC: u32 = 0x3084_0001;
/// `mullw r4, r5, r6`.
const MULLW: u32 = 0x7c85_31d6;

/// `body` closed by its `bdnz`.
fn looped(body: &[u32]) -> &'static [u32] {
    let mut words = body.to_vec();
    let back = (4 * words.len() as u32).wrapping_neg() & 0xfffc;
    words.push(0x4200_0000 | back);
    Box::leak(words.into_boxed_slice())
}

/// Each loop's body before its `bdnz`, in the probe's order.
fn bodies() -> [Vec<u32>; 10] {
    let li8: Vec<u32> = (4..12).map(li).collect();
    [
        (3..11).map(mfxer).collect(),
        vec![mfxer(10); 8],
        [&[mfxer(3)][..], &li8].concat(),
        vec![mfxer(3), ADDI_USE],
        [&li8[..], &li8].concat(),
        vec![mtxer(14), mfxer(3)],
        vec![ADDIC, mfxer(3)],
        (3..11).map(mfcr).collect(),
        vec![mtxer(14); 8],
        [MULLW, mfxer(3)].repeat(4),
    ]
}

/// CellGov's cycles an iteration for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let r = Replay {
        loop_index: k,
        words: looped(&bodies()[k]),
        seeds: &[Seed::Gpr(14, 0)],
    };
    replay(&r, 0).expect("the loop replays")
}

/// The share of a loop's fastest console run the model may lie on either
/// side of it: the `mfxer` and dependent `addi` loop lies 7.2% under.
const BAND: f64 = 0.10;

#[test]
fn each_mfxer_mtxer_and_mfcr_loop_lies_within_the_band() {
    let w = console_words();
    let rows: Vec<(usize, f64, f64)> = (0..bodies().len())
        .map(|k| (k, cellgov_cycles(k), console_cycles(&w, k)))
        .collect();
    assert_eq!(rows.len(), 10);
    for &(k, cellgov, console) in &rows {
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}: {rows:?}",
        );
    }
}
