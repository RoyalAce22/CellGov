//! CellGov's PPU timing against the `ppu_issue_barrier_forms` console
//! capture: what a `sync` of each L form costs, and what a `sync`,
//! `lwsync` or `isync` costs after a store of several store-queue
//! entries.
//!
//! The probe times eighteen loops with `mftb`, five runs each. Every
//! store addresses one data line, which r29 holds.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_barrier_forms/ps3/cech20-cex-493/observation.json";

/// The loops the probe times.
const LOOPS: usize = 18;

/// The payload's word 38 when the probe wrote every word.
const DONE: u64 = 0x5050_4246;

/// log2 of each loop's iterations on the console.
const ITERATIONS_LOG2: u32 = 16;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 312);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[2 * LOOPS + 2], DONE, "the probe wrote every word");
    assert_eq!(
        (words[2 * LOOPS], words[2 * LOOPS + 1]),
        (5, u64::from(ITERATIONS_LOG2))
    );
    words
}

/// Loop `k`'s fastest run, in core cycles an iteration.
fn console_cycles(words: &[u64], k: usize) -> f64 {
    (words[2 * k] * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64
}

/// `std r14, d(r29)`.
const fn std(d: u32) -> u32 {
    0xf9dd_0000 | d
}
/// `stmw r24, 0(r29)`: 32 bytes, two 16-byte blocks.
const STMW24: u32 = 0xbf1d_0000;
/// `stmw r16, 0(r29)`: 64 bytes, four 16-byte blocks.
const STMW16: u32 = 0xbe1d_0000;
/// `sync`.
const SYNC: u32 = 0x7c00_04ac;
/// `ptesync`: `sync` with L = 2.
const PTESYNC: u32 = 0x7c40_04ac;
/// `sync 3`.
const SYNC3: u32 = 0x7c60_04ac;
/// `lwsync`.
const LWSYNC: u32 = 0x7c20_04ac;
/// `isync`.
const ISYNC: u32 = 0x4c00_012c;
/// `std r14, 12(r29)`: bytes 12 to 19, two 16-byte blocks.
const STD12: u32 = std(12);
/// Four `std`, one to each of four 16-byte blocks.
const STD4: [u32; 4] = [std(0), std(16), std(32), std(48)];

/// Each loop's body before its `bdnz`, in the probe's order.
fn bodies() -> [Vec<u32>; LOOPS] {
    let with = |stores: &[u32], barrier: u32| -> Vec<u32> {
        stores.iter().copied().chain([barrier]).collect()
    };
    [
        vec![SYNC],
        vec![SYNC3],
        with(&[std(0)], SYNC),
        with(&[std(0)], SYNC3),
        vec![LWSYNC],
        with(&[std(0)], LWSYNC),
        with(&[STD12], SYNC),
        with(&[STD12], LWSYNC),
        with(&[STMW24], SYNC),
        with(&[STMW24], LWSYNC),
        with(&[STMW16], SYNC),
        with(&[STMW16], LWSYNC),
        with(&STD4, SYNC),
        with(&STD4, LWSYNC),
        with(&[STD12], ISYNC),
        with(&[STMW16], ISYNC),
        vec![PTESYNC],
        with(&[std(0)], PTESYNC),
    ]
}

/// CellGov's cycles an iteration for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let mut words = bodies()[k].clone();
    words.push(0x4200_0000 | ((4 * words.len() as u32).wrapping_neg() & 0xfffc));
    let r = Replay {
        loop_index: k,
        words: Box::leak(words.into_boxed_slice()),
        seeds: &[Seed::Gpr(29, ONE_LINE)],
    };
    replay(&r, 0).expect("the loop replays")
}

/// The share of a loop's fastest console run the model may lie on either
/// side of it, as for the barrier loops of `ppu_issue_fp_store_drain`.
const BAND: f64 = 0.06;

/// The loops of an `stmw` before a barrier that the model times outside
/// the band.
const STORE_MULTIPLE: [usize; 4] = [8, 9, 11, 15];

/// The loops of `sync 3` and `ptesync`, alone and after an `std`, each
/// with the `sync` loop of the same shape.
const SYNC_FORMS: [(usize, usize); 4] = [(1, 0), (3, 2), (16, 0), (17, 2)];

#[test]
fn every_barrier_loop_but_an_stmw_before_one_lies_within_the_band() {
    let w = console_words();
    let rows: Vec<(usize, f64, f64)> = (0..LOOPS)
        .filter(|k| !STORE_MULTIPLE.contains(k))
        .map(|k| (k, cellgov_cycles(k), console_cycles(&w, k)))
        .collect();
    assert_eq!(rows.len(), LOOPS - STORE_MULTIPLE.len());
    for &(k, cellgov, console) in &rows {
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}: {rows:?}",
        );
    }
}

#[test]
fn a_sync_3_and_a_ptesync_cost_the_model_a_sync() {
    for (k, sync) in SYNC_FORMS {
        assert_eq!(
            cellgov_cycles(k),
            cellgov_cycles(sync),
            "loop {k} against loop {sync}"
        );
    }
}

/// A barrier after an `stmw` costs the console about 41 cycles more than
/// after an `std`, whatever the register count. The model drains the
/// `stmw`'s microwords before the barrier, so it falls short.
#[test]
fn an_stmw_before_a_barrier_is_a_known_difference() {
    let w = console_words();
    for k in STORE_MULTIPLE {
        let (cellgov, console) = (cellgov_cycles(k), console_cycles(&w, k));
        assert!(
            (console - cellgov) / console > BAND,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration now lies within the band \
             of the console's {console:.2}; move the loop into the band set",
        );
    }
}
