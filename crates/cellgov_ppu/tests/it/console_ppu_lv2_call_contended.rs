//! The `ppu_lv2_call_contended` console capture against the round trips
//! CellGov charges: five loops of 4096 calls or handoffs, five runs each.
//!
//! The probe times these loops, and each helper runs at the caller's
//! priority:
//!
//! 0. `sys_time_get_timebase_frequency`, the cheapest call;
//! 1. `sys_ppu_thread_yield` with no helper;
//! 2. `sys_ppu_thread_yield` with one helper that yields;
//! 3. `sys_ppu_thread_yield` with two helpers that yield;
//! 4. a semaphore handoff, whose every wait blocks until the helper posts.
//!
//! The PPU runs two hardware threads, so only the loop with two helpers
//! switches threads at a yield.

use crate::console_ppu_issue_timing::replay_r3;
use crate::console_ppu_lv2_call_cost::{call_replay, YIELD_LOOP};
use cellgov_ps3_abi::hw::ppu_issue::{
    ppu_syscall_round_trip_cycles, PPU_CYCLES_PER_TB_TICK, PPU_SYSCALL_ROUND_TRIP_CYCLES,
};
use cellgov_ps3_abi::lv2::syscall as sc;

const CONSOLE: &str =
    "../../tests/micro/ppu_lv2_call_contended/ps3/cech20-cex-493/observation.json";

/// The payload's word 60 when the probe wrote every word.
const DONE: u64 = 0x5050_4C59;

/// Calls or handoffs a loop makes.
const CALLS: u64 = 1 << 12;

/// The runs of each loop.
const REPS: u64 = 5;

const FREQUENCY_LOOP: usize = 0;
const ALONE_LOOP: usize = 1;
const SIBLING_LOOP: usize = 2;
const SWITCH_LOOP: usize = 3;
const HANDOFF_LOOP: usize = 4;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 512);
    let w: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(w[60], DONE, "the probe wrote every word");
    assert_eq!(w[59], REPS, "five runs a loop");
    assert_eq!(w[57], 0, "every setup and teardown step returned 0");
    for (word, &rc) in (32..).zip(&w[32..44]) {
        assert_eq!(rc, 0, "step word {word}");
    }
    w
}

/// Loop `l`'s fastest and slowest runs, in core cycles a call.
fn console_cycles(w: &[u64], l: usize) -> (f64, f64) {
    let per = |ticks: u64| (ticks * PPU_CYCLES_PER_TB_TICK) as f64 / CALLS as f64;
    (per(w[2 * l]), per(w[2 * l + 1]))
}

/// The cycles of the probe's loop beside the cheapest call's round trip.
fn rest(w: &[u64]) -> f64 {
    console_cycles(w, FREQUENCY_LOOP).0.round() - PPU_SYSCALL_ROUND_TRIP_CYCLES as f64
}

#[test]
fn every_helper_ran_and_every_handoff_completed() {
    let w = console_words();
    assert!(w[48] > 0, "the first yielding helper yielded");
    assert!(w[49] > 0, "the second yielding helper yielded");
    assert_eq!(
        w[50],
        REPS * CALLS,
        "the handoff helper answered every post"
    );
    assert_eq!(w[16 + HANDOFF_LOOP], 0, "no handoff post or wait failed");
    assert_eq!(w[24 + HANDOFF_LOOP], 0, "the last handoff wait returned 0");
}

#[test]
fn a_yield_with_no_thread_waiting_leaves_1_in_r3() {
    // With one helper the two threads hold both hardware threads, so no
    // yield finds a thread waiting. With two helpers some yields switch
    // threads, and the loop's last yield left 0.
    let w = console_words();
    for l in [ALONE_LOOP, SIBLING_LOOP] {
        assert_eq!(w[16 + l], 1, "loop {l}: the first nonzero r3");
        assert_eq!(w[24 + l], 1, "loop {l}: the last r3");
    }
    assert_eq!(
        w[16 + SWITCH_LOOP],
        1,
        "a switching loop's first nonzero r3"
    );
    assert_eq!(w[24 + SWITCH_LOOP], 0, "the switching loop's last r3");
}

#[test]
fn cellgov_leaves_0_in_r3_where_the_console_s_uncontended_yield_leaves_1() {
    // The `ppu_lv2_call_cost` capture's yield loop, replayed alone.
    let w = console_words();
    assert_eq!(w[24 + ALONE_LOOP], 1, "the console's last uncontended r3");
    let r3 = replay_r3(&call_replay(YIELD_LOOP)).expect("the loop replays");
    assert_eq!(
        r3, 0,
        "CellGov's answer changed: hold it to the console's 1"
    );
}

#[test]
fn the_uncontended_yield_lies_within_its_loop_s_spread_of_the_table() {
    let w = console_words();
    let (fastest, slowest) = console_cycles(&w, ALONE_LOOP);
    let table = ppu_syscall_round_trip_cycles(sc::PPU_THREAD_YIELD) as f64;
    assert!(
        (fastest - rest(&w) - table).abs() <= slowest - fastest,
        "the yield's round trip {:.0} against the table's {table:.0}, spread {:.0}",
        fastest - rest(&w),
        slowest - fastest,
    );
}

#[test]
fn contended_calls_cost_the_console_more_than_the_uncontended_trips_cellgov_charges() {
    // CellGov charges a yield its uncontended trip whatever the other
    // threads do. It charges a woken wait the cheapest call's trip from
    // the poster's commit. The console's contended paths cost several
    // times that, and these ratios hold the size of the gap.
    let w = console_words();
    let yield_trip = ppu_syscall_round_trip_cycles(sc::PPU_THREAD_YIELD) as f64;
    let sibling = console_cycles(&w, SIBLING_LOOP).0 - rest(&w);
    let switch = console_cycles(&w, SWITCH_LOOP).0 - rest(&w);
    assert!(
        (1.5..2.0).contains(&(sibling / yield_trip)),
        "a yield beside a yielding sibling: {sibling:.0} against {yield_trip:.0}",
    );
    assert!(
        (5.5..6.5).contains(&(switch / yield_trip)),
        "a yield among three threads: {switch:.0} against {yield_trip:.0}",
    );
    // A handoff round is two posts and two waits. CellGov's waiter
    // resumes at the post's commit, so its round is at most the four
    // trips one after another.
    let handoff = console_cycles(&w, HANDOFF_LOOP).0;
    assert_eq!(
        ppu_syscall_round_trip_cycles(sc::SEMAPHORE_WAIT),
        PPU_SYSCALL_ROUND_TRIP_CYCLES,
        "a wait takes the cheapest call's trip",
    );
    let serial = 2.0
        * (ppu_syscall_round_trip_cycles(sc::SEMAPHORE_POST)
            + ppu_syscall_round_trip_cycles(sc::SEMAPHORE_WAIT)) as f64;
    assert!(
        (7.0..7.6).contains(&(handoff / serial)),
        "a handoff round: {handoff:.0} against CellGov's {serial:.0} for four serial trips",
    );
}
