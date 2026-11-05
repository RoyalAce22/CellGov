//! CellGov's PPU timing against the `ppu_issue_fp_store_barrier` console
//! capture: a barrier after a single-precision store against one after a
//! double-precision store, an integer store and an `frsp` with a
//! double-precision store.
//!
//! The probe times fourteen loops with `mftb`, eleven runs each. Every
//! store addresses one data line, which r29 holds. f14 holds 0.0, f15
//! holds 1.5 and r14 holds 0.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_fp_store_barrier/ps3/cech20-cex-493/observation.json";

/// The loops the probe times.
const LOOPS: usize = 14;

/// The payload's word 30 when the probe wrote every word.
const DONE: u64 = 0x5050_4653;

/// The runs the probe takes of each loop.
const REPS: u64 = 11;

/// log2 of each loop's iterations on the console.
const ITERATIONS_LOG2: u32 = 16;

/// The payload of the capture at `path` as big-endian words.
fn words_at(path: &str) -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 8 * (2 * LOOPS + 3));
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[2 * LOOPS + 2], DONE, "the probe wrote every word");
    assert_eq!(
        (words[2 * LOOPS], words[2 * LOOPS + 1]),
        (REPS, u64::from(ITERATIONS_LOG2))
    );
    words
}

/// Loop `k`'s fastest and slowest runs, in core cycles an iteration.
fn console_cycles(words: &[u64], k: usize) -> (f64, f64) {
    let per = |t: u64| (t * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64;
    (per(words[2 * k]), per(words[2 * k + 1]))
}

/// `std r14, 0(r29)`.
const STD: u32 = 0xf9dd_0000;
/// `stw r14, 0(r29)`.
const STW: u32 = 0x91dd_0000;
/// `stfd f14, 0(r29)`.
const STFD: u32 = 0xd9dd_0000;
/// `stfs f14, 0(r29)`.
const STFS: u32 = 0xd1dd_0000;
/// `stfs f15, 0(r29)`.
const STFS15: u32 = 0xd1fd_0000;
/// `stfs f14, 4(r29)`.
const STFS4: u32 = 0xd1dd_0004;
/// `stfiwx f14, 0, r29`.
const STFIWX: u32 = 0x7dc0_efae;
/// `frsp f16, f14`.
const FRSP: u32 = 0xfe00_7018;
/// `stfd f16, 0(r29)`.
const STFD16: u32 = 0xda1d_0000;
/// `sync`.
const SYNC: u32 = 0x7c00_04ac;
/// `lwsync`.
const LWSYNC: u32 = 0x7c20_04ac;
/// `isync`.
const ISYNC: u32 = 0x4c00_012c;
/// `addi rn, rn, 1` for r3 through r10.
const ADDI8: [u32; 8] = [
    0x3863_0001,
    0x3884_0001,
    0x38a5_0001,
    0x38c6_0001,
    0x38e7_0001,
    0x3908_0001,
    0x3929_0001,
    0x394a_0001,
];

/// Each loop's body before its `bdnz`, in the probe's order.
fn bodies() -> [Vec<u32>; LOOPS] {
    let with_addi = |store: u32| [&[store][..], &ADDI8, &[SYNC]].concat();
    [
        vec![STD, SYNC],
        vec![STFD, SYNC],
        vec![STFS, SYNC],
        vec![STFIWX, SYNC],
        vec![STW, SYNC],
        vec![STFS15, SYNC],
        vec![STFD, LWSYNC],
        vec![STFS, LWSYNC],
        vec![STFD, ISYNC],
        vec![STFS, ISYNC],
        with_addi(STFS),
        with_addi(STFD),
        vec![STFS4, SYNC],
        vec![FRSP, STFD16, SYNC],
    ]
}

/// The registers the probe sets before each loop.
const SEEDS: &[Seed] = &[
    Seed::Gpr(29, ONE_LINE),
    Seed::Gpr(14, 0),
    Seed::Fpr(14, 0),
    Seed::Fpr(15, 0x3ff8_0000_0000_0000),
];

/// CellGov's cycles an iteration for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let mut words = bodies()[k].clone();
    words.push(0x4200_0000 | ((4 * words.len() as u32).wrapping_neg() & 0xfffc));
    let r = Replay {
        loop_index: k,
        words: Box::leak(words.into_boxed_slice()),
        seeds: SEEDS,
    };
    replay(&r, 0).expect("the loop replays")
}

/// The share of a loop's fastest console run the model may lie on either
/// side of it, as for the barrier loops of `ppu_issue_fp_store_drain`.
const BAND: f64 = 0.06;

/// The loops the model times outside the band, each with its bound.
///
/// A bound is the share of the console's fastest run that the model may
/// lie over. The two loops are `stfd; lwsync` and `stfs; lwsync`. This
/// capture times both at 83 cycles an iteration, and the model times both
/// at 89. The `ppu_issue_fp_store_drain` capture's `stfd; lwsync` loop
/// lies within the band of the model.
const RESIDUALS: [(usize, f64); 2] = [(6, 0.09), (7, 0.09)];

#[test]
fn each_fp_store_barrier_loop_lies_within_the_band_or_its_named_residual() {
    let w = words_at(CONSOLE);
    let rows: Vec<(usize, f64, f64)> = (0..LOOPS)
        .map(|k| (k, cellgov_cycles(k), console_cycles(&w, k).0))
        .collect();
    for &(k, cellgov, console) in &rows {
        let over = (cellgov - console) / console;
        match RESIDUALS.iter().find(|r| r.0 == k) {
            Some(&(_, bound)) => {
                assert!(
                    over > BAND,
                    "loop {k} now lies within the band; drop it from the residuals: {rows:?}",
                );
                assert!(
                    over < bound,
                    "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's \
                     fastest {console:.2}, more than {bound} of it over",
                );
            }
            None => assert!(
                over.abs() <= BAND,
                "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
                 {console:.2}: {rows:?}",
            ),
        }
    }
}

/// The loops of an `stfd`, an `stfs`, an `stfiwx` and an `stfs` of 1.5
/// before a `sync`.
const FP_STORE_SYNC: [usize; 4] = [1, 2, 3, 5];

#[test]
fn every_fp_store_form_costs_a_sync_the_same() {
    let w = words_at(CONSOLE);
    let stfd = console_cycles(&w, FP_STORE_SYNC[0]).0;
    for k in FP_STORE_SYNC {
        let console = console_cycles(&w, k).0;
        assert!(
            (console - stfd).abs() < 2.0,
            "loop {k}: the console {console:.2}, its stfd loop {stfd:.2}",
        );
        assert_eq!(
            cellgov_cycles(k),
            cellgov_cycles(FP_STORE_SYNC[0]),
            "loop {k}"
        );
    }
}

/// The `ppu_issue_fp_store_drain` capture, whose loop 3 is this probe's
/// loop 2.
const FP_STORE_DRAIN: &str =
    "../../tests/micro/ppu_issue_fp_store_drain/ps3/cech20-cex-493/observation.json";

/// The `stfs; sync` loop's fastest run in the `ppu_issue_fp_store_drain`
/// capture, in core cycles an iteration.
fn fp_store_drain_stfs_sync() -> f64 {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(FP_STORE_DRAIN))
        .unwrap_or_else(|e| panic!("{FP_STORE_DRAIN}: {e}"));
    let p = &observation.memory_regions[0].data;
    let at = 8 * 2 * 3;
    let ticks = u64::from_be_bytes(p[at..at + 8].try_into().expect("8 bytes"));
    (ticks * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64
}

#[test]
fn the_fp_store_drain_capture_timed_the_stfs_sync_loop_slower_than_this_one() {
    let here = console_cycles(&words_at(CONSOLE), 2).0;
    let there = fp_store_drain_stfs_sync();
    let cellgov = cellgov_cycles(2);
    assert!(
        there > here * (1.0 + BAND),
        "the two captures time stfs; sync at {here:.2} and {there:.2}",
    );
    assert!(
        (cellgov - here).abs() / here <= BAND,
        "CellGov {cellgov:.2}"
    );
    assert!((there - cellgov) / there > BAND, "CellGov {cellgov:.2}");
}
