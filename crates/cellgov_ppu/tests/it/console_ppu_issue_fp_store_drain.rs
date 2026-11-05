//! CellGov's PPU timing against the `ppu_issue_fp_store_drain` console
//! capture: whether a `sync`, `lwsync`, `isync`, `mffs`, `mfvscr` or `sc`
//! waits longer after an FP store than after a fixed-point store.
//!
//! The probe times twenty loops with `mftb`, five runs each. Every store
//! addresses one data line, which r29 holds.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::{PPU_CYCLES_PER_TB_TICK, PPU_LWSYNC_CYCLES, PPU_SYNC_CYCLES};

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_fp_store_drain/ps3/cech20-cex-493/observation.json";

/// The payload's word 46 when the probe wrote every word.
const DONE: u64 = 0x5050_4653;

/// log2 of each loop's iterations on the console: the probe's `ITER`, and
/// its `SC_ITER` for the loops of calls.
const ITERATIONS_LOG2: u32 = 16;
const SC_ITERATIONS_LOG2: u32 = 12;

/// The first loop of calls.
const FIRST_CALL_LOOP: usize = 17;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 376);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[46], DONE, "the probe wrote every word");
    assert_eq!(
        (words[40], words[41], words[42]),
        (5, u64::from(ITERATIONS_LOG2), u64::from(SC_ITERATIONS_LOG2))
    );
    assert_eq!(
        words[43..46],
        [cellgov_ps3_abi::hw::ppu::CELL_PPU_TIMEBASE_HZ; 3],
        "words 43 to 45: each loop of calls returns the time-base frequency"
    );
    words
}

/// Loop `k`'s fastest run, in core cycles an iteration.
fn console_cycles(words: &[u64], k: usize) -> f64 {
    let log2 = if k >= FIRST_CALL_LOOP {
        SC_ITERATIONS_LOG2
    } else {
        ITERATIONS_LOG2
    };
    (words[2 * k] * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << log2) as f64
}

/// `std r14, 0(r29)`.
const STD: u32 = 0xf9dd_0000;
/// `stfd f14, 0(r29)`.
const STFD: u32 = 0xd9dd_0000;
/// `stfs f14, 0(r29)`.
const STFS: u32 = 0xd1dd_0000;
/// `stfiwx f14, 0, r29`.
const STFIWX: u32 = 0x7dc0_efae;
/// `sync`.
const SYNC: u32 = 0x7c00_04ac;
/// `lwsync`.
const LWSYNC: u32 = 0x7c20_04ac;
/// `isync`.
const ISYNC: u32 = 0x4c00_012c;
/// `mffs f3`.
const MFFS: u32 = 0xfc60_048e;
/// `mfvscr v3`.
const MFVSCR: u32 = 0x1060_0604;
/// `li r11, 147; sc`: `sys_time_get_timebase_frequency`.
const CALL: [u32; 2] = [0x3960_0093, 0x4400_0002];
/// `addi r28, r28, -1; cmpdi r28, 0`, before the `bne` that closes a loop
/// of calls.
const COUNT_DOWN: [u32; 2] = [0x3b9c_ffff, 0x2c3c_0000];

/// `body` closed by its `bdnz`, or for a loop of calls by its count down
/// in r28 and `bne`.
fn looped(k: usize, body: &[u32]) -> &'static [u32] {
    let mut words = body.to_vec();
    let back = |len: usize| (4 * len as u32).wrapping_neg() & 0xfffc;
    if k >= FIRST_CALL_LOOP {
        words.extend_from_slice(&CALL);
        words.extend_from_slice(&COUNT_DOWN);
        words.push(0x4082_0000 | back(words.len()));
    } else {
        words.push(0x4200_0000 | back(words.len()));
    }
    Box::leak(words.into_boxed_slice())
}

/// Each loop's body before its close, in the probe's order.
fn bodies() -> [Vec<u32>; 20] {
    [
        vec![SYNC],
        vec![STD, SYNC],
        vec![STFD, SYNC],
        vec![STFS, SYNC],
        vec![STFIWX, SYNC],
        vec![LWSYNC],
        vec![STD, LWSYNC],
        vec![STFD, LWSYNC],
        vec![ISYNC],
        vec![STD, ISYNC],
        vec![STFD, ISYNC],
        vec![MFFS],
        vec![STD, MFFS],
        vec![STFD, MFFS],
        vec![MFVSCR],
        vec![STD, MFVSCR],
        vec![STFD, MFVSCR],
        vec![],
        vec![STD],
        vec![STFD],
    ]
}

/// CellGov's cycles an iteration for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let seeds: &'static [Seed] = if k >= FIRST_CALL_LOOP {
        &[Seed::Gpr(29, ONE_LINE), Seed::Counter(28)]
    } else {
        &[Seed::Gpr(29, ONE_LINE)]
    };
    let r = Replay {
        loop_index: k,
        words: looped(k, &bodies()[k]),
        seeds,
    };
    replay(&r, 0).expect("the loop replays")
}

/// The loops of an `std` and of an `stfd` before an `lwsync`, an `isync`
/// and an `sc`.
const FP_STORE_EXTRA: [(usize, usize); 3] = [(6, 7), (9, 10), (18, 19)];

/// The cycles an iteration the model's extra for an `stfd` over an `std`
/// may lie from the console's. The `lwsync` pair's lies 4.0 over, and its
/// `std` loop's five runs spread from 69.5 to 95.6 cycles.
const EXTRA_GAP: f64 = 4.5;

#[test]
fn an_fp_store_costs_an_lwsync_isync_or_sc_the_console_s_extra_within_the_gap() {
    let w = console_words();
    for (fixed, fp) in FP_STORE_EXTRA {
        let cellgov = cellgov_cycles(fp) - cellgov_cycles(fixed);
        let console = console_cycles(&w, fp) - console_cycles(&w, fixed);
        assert!(
            (cellgov - console).abs() <= EXTRA_GAP,
            "loops {fixed} and {fp}: the stfd costs CellGov {cellgov:.2} cycles an \
             iteration more than the std, the console {console:.2}",
        );
    }
}

/// The share of a loop's fastest console run the model may lie on either
/// side of it, for the loops of calls: the `std` loop lies 0.6% under.
const CALL_BAND: f64 = 0.01;

#[test]
fn each_loop_of_calls_lies_within_the_band() {
    let w = console_words();
    for k in FIRST_CALL_LOOP..bodies().len() {
        let (cellgov, console) = (cellgov_cycles(k), console_cycles(&w, k));
        assert!(
            (cellgov - console).abs() / console <= CALL_BAND,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}",
        );
    }
}

/// The loops of an `mffs` and of an `mfvscr`, alone and after each store,
/// except the `stfd` before an `mffs`.
const HELD: [usize; 5] = [11, 12, 14, 15, 16];

/// The share of a loop's fastest console run the model may lie on either
/// side of it.
const BAND: f64 = 0.06;

#[test]
fn each_mffs_loop_and_each_mfvscr_loop_lies_within_the_band() {
    let w = console_words();
    for k in HELD {
        let (cellgov, console) = (cellgov_cycles(k), console_cycles(&w, k));
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}",
        );
    }
}

/// The loops of a `sync`, an `lwsync` or an `isync`, alone and after
/// each store, except the `stfs` before a `sync`.
const BARRIERS: [usize; 10] = [0, 1, 2, 4, 5, 6, 7, 8, 9, 10];

#[test]
fn each_sync_lwsync_and_isync_loop_lies_within_the_band() {
    let w = console_words();
    for k in BARRIERS {
        let (cellgov, console) = (cellgov_cycles(k), console_cycles(&w, k));
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}",
        );
    }
}

/// The loops of a `sync` alone and of an `lwsync` alone. Each barrier
/// starts a dispatch group, so its `bdnz` is the second instruction of
/// that group.
const ALONE_BARRIERS: [(usize, u64); 2] = [(0, PPU_SYNC_CYCLES), (5, PPU_LWSYNC_CYCLES)];

#[test]
fn the_bdnz_after_a_barrier_in_its_group_waits_out_the_barrier() {
    for (k, hold) in ALONE_BARRIERS {
        let cellgov = cellgov_cycles(k);
        assert!(
            cellgov >= (hold + 1) as f64,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, under the barrier's \
             {hold} and its bdnz",
        );
    }
}

/// The loop of an `stfs` before a `sync`, and of an `stfd` before one.
const STFS_SYNC: usize = 3;
const STFD_SYNC: usize = 2;

/// This capture times an `stfs` before a `sync` about 27 cycles slower
/// than an `stfd` before one, and the model times them alike. Two runs
/// of the `ppu_issue_fp_store_barrier` probe, each at 11 runs a loop,
/// time the two stores within 2 cycles of each other.
#[test]
fn an_stfs_before_a_sync_is_a_known_difference() {
    let w = console_words();
    let (stfs, stfd) = (cellgov_cycles(STFS_SYNC), cellgov_cycles(STFD_SYNC));
    assert_eq!(stfs, stfd, "the model times the two stores alike");
    let console = console_cycles(&w, STFS_SYNC);
    assert!(
        (console - stfs) / console > BAND,
        "loop {STFS_SYNC}: CellGov {stfs:.2} cycles an iteration now lies within the band \
         of the console's {console:.2}; move the loop into the barrier set",
    );
}

/// The loop of an `stfd` before an `mffs`.
const STFD_MFFS: usize = 13;

/// The console times this loop at 32.00 cycles an iteration, and a second
/// run of the probe at 34.16. The model holds the `stfd` behind the older
/// `mffs` and gives 40. In the `ppu_issue_store_drain_distance` capture,
/// an `stfd` 16 or 32 `li` after the older `mffs` makes the younger `mffs`
/// wait until 42 cycles after the older one, as the model gives.
#[test]
fn an_stfd_just_before_an_mffs_is_a_known_difference() {
    let w = console_words();
    let (cellgov, console) = (cellgov_cycles(STFD_MFFS), console_cycles(&w, STFD_MFFS));
    assert!(
        (cellgov - console) / console > BAND,
        "loop {STFD_MFFS}: CellGov {cellgov:.2} cycles an iteration now lies within the band \
         of the console's {console:.2}; move the loop into the held set",
    );
}
