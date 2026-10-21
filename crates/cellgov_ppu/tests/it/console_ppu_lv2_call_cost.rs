//! CellGov's LV2 round trips against the `ppu_lv2_call_cost` console
//! capture: fifteen loops of 4096 uncontended calls, five runs each.
//!
//! Every loop runs the probe's `calls` body: the four arguments from
//! r26 to r29, zeroes in r7 to r10, the number from r25, the `sc`, the
//! return-code check on cr4, and the count in r31. The replay starts
//! with cr4 clear, so it skips the check's two instructions, which the
//! console runs while every call so far returned 0.

use crate::console_ppu_issue_timing::{replay, Replay, Seed};
use cellgov_ps3_abi::hw::ppu_issue::{
    ppu_syscall_round_trip_cycles, PPU_CYCLES_PER_TB_TICK, PPU_SYSCALL_ROUND_TRIPS,
    PPU_SYSCALL_ROUND_TRIP_CYCLES,
};
use cellgov_ps3_abi::lv2::syscall as sc;

const CONSOLE: &str = "../../tests/micro/ppu_lv2_call_cost/ps3/cech20-cex-493/observation.json";

/// The payload's word 60 when the probe wrote every word.
const DONE: u64 = 0x5050_4C43;

/// Calls a loop makes.
const CALLS: u64 = 1 << 12;

/// The syscall each loop times, in loop order.
const LOOPS: [u64; 15] = [
    sc::TIME_GET_TIMEBASE_FREQUENCY,
    sc::TIME_GET_CURRENT_TIME,
    sc::MEMORY_GET_USER_MEMORY_SIZE,
    sc::PPU_THREAD_YIELD,
    sc::SEMAPHORE_GET_VALUE,
    sc::SEMAPHORE_POST,
    sc::SEMAPHORE_TRY_WAIT,
    sc::EVENT_FLAG_GET,
    sc::EVENT_FLAG_SET,
    sc::EVENT_FLAG_CLEAR,
    sc::MUTEX_LOCK,
    sc::MUTEX_UNLOCK,
    sc::MUTEX_TRYLOCK,
    sc::MUTEX_UNLOCK,
    sc::EVENT_QUEUE_TRY_RECEIVE,
];

/// The loop of [`LOOPS`] whose call returns the time-base frequency in r3.
const FREQUENCY_LOOP: usize = 0;

/// The loop of `sys_ppu_thread_yield`, which leaves 1 in r3.
const YIELD_LOOP: usize = 3;

/// The cycles the model's replay of the cheapest call's loop runs over
/// the console's fastest run.
const LOOP_GAP: f64 = 13.0;

/// The second loop of `sys_mutex_unlock`, a repeat of loop 11.
const REPEAT_LOOP: usize = 13;

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
    assert_eq!(w[59], 5, "five runs a loop");
    for (word, what) in (48..56).zip([
        "semaphore create",
        "event flag create",
        "mutex create",
        "event queue create",
        "semaphore destroy",
        "event flag destroy",
        "mutex destroy",
        "event queue destroy",
    ]) {
        assert_eq!(w[word], 0, "{what}");
    }
    assert_eq!(
        w[32 + FREQUENCY_LOOP],
        w[56],
        "loop 0 returns the frequency"
    );
    assert_eq!(w[32 + YIELD_LOOP], 1, "a yield leaves 1 in r3");
    for l in (0..LOOPS.len()).filter(|&l| l != FREQUENCY_LOOP && l != YIELD_LOOP) {
        assert_eq!(w[32 + l], 0, "every call of loop {l} returned 0");
    }
    assert_eq!(w[57], 0, "the semaphore value before the posts");
    assert_eq!(w[58], 0, "the empty queue's event count");
    w
}

/// Loop `l`'s fastest and slowest runs, in core cycles a call.
fn console_cycles(w: &[u64], l: usize) -> (f64, f64) {
    let per = |ticks: u64| (ticks * PPU_CYCLES_PER_TB_TICK) as f64 / CALLS as f64;
    (per(w[2 * l]), per(w[2 * l + 1]))
}

/// The probe's `calls` loop body, from its `mr r3,r26` to its `bne+`.
const CALL_LOOP: [u32; 16] = [
    0x7f43_d378,
    0x7f64_db78,
    0x7f85_e378,
    0x7fa6_eb78,
    0x38e0_0000,
    0x3900_0000,
    0x3920_0000,
    0x3940_0000,
    0x7f2b_cb78,
    0x4400_0002,
    0x4092_000c,
    0x787e_0020,
    0x2e3e_0000,
    0x2c3f_0001,
    0x3bff_ffff,
    0x4082_ffc4,
];

/// Each loop's seeds: its syscall number in r25 and the count in r31.
/// The arguments stay zero: a replay times the round trip, and no LV2
/// host answers the call.
const SEEDS: [[Seed; 2]; 15] = {
    let mut s = [[Seed::Counter(31); 2]; 15];
    let mut l = 0;
    while l < LOOPS.len() {
        s[l][0] = Seed::Gpr(25, LOOPS[l]);
        l += 1;
    }
    s
};

/// The replay of loop `l`.
fn call_replay(l: usize) -> Replay {
    Replay {
        loop_index: l,
        words: &CALL_LOOP,
        seeds: &SEEDS[l],
    }
}

#[test]
fn each_timed_call_s_round_trip_is_its_fastest_run_less_the_loop_s_other_cycles() {
    let w = console_words();
    // The rest of the loop beside the cheapest call's round trip.
    let rest = console_cycles(&w, FREQUENCY_LOOP).0.round() as u64 - PPU_SYSCALL_ROUND_TRIP_CYCLES;
    assert_eq!(rest, 13, "the loop's own cycles");
    for (l, &number) in LOOPS.iter().enumerate() {
        let fastest = console_cycles(&w, l).0.round() as u64;
        if l == REPEAT_LOOP {
            let table = ppu_syscall_round_trip_cycles(number);
            assert!(
                (fastest - rest).abs_diff(table) <= 2,
                "loop {l} repeats loop 11 to 2 cycles: {} against {table}",
                fastest - rest,
            );
            continue;
        }
        assert_eq!(
            ppu_syscall_round_trip_cycles(number),
            fastest - rest,
            "the round trip of syscall {number}, loop {l}",
        );
    }
    let mut timed: Vec<u64> = PPU_SYSCALL_ROUND_TRIPS.iter().map(|&(n, _)| n).collect();
    let mut looped: Vec<u64> = LOOPS.to_vec();
    timed.sort_unstable();
    looped.sort_unstable();
    looped.dedup();
    assert_eq!(timed, looped, "the table holds every timed call once");
}

#[test]
fn a_call_that_writes_a_result_costs_more_than_one_that_writes_none() {
    // The capture's shape, held on the table: every call writing a
    // result to guest memory costs over twice the slowest object call
    // that writes none, and the cheapest call is the time-base frequency.
    let fastest = |l: usize| ppu_syscall_round_trip_cycles(LOOPS[l]) as f64;
    let writes = [1, 2, 4, 7, 14];
    let objects = [5, 6, 8, 9, 10, 11, 12, 13];
    let slowest_object = objects.iter().map(|&l| fastest(l)).fold(0.0, f64::max);
    for l in writes {
        assert!(
            fastest(l) > 2.0 * slowest_object,
            "loop {l}: {:.0} against {slowest_object:.0}",
            fastest(l),
        );
    }
    for l in 1..LOOPS.len() {
        assert!(fastest(l) > fastest(FREQUENCY_LOOP), "loop {l}");
    }
}

#[test]
fn the_issue_model_times_each_call_loop_within_the_console_s_spread() {
    let w = console_words();
    // An accepted gap: the model's own cycles for the loop around the
    // cheapest call are 26 against the console's 13, so the model times
    // that loop 13 cycles slow. The `ppu_issue_timing` loop of the same
    // call fits the 929 round trip with 20 cycles of loop, so the split
    // between the call and its loop moves with the probe.
    let cheapest = replay(&call_replay(FREQUENCY_LOOP), 0).expect("the loop replays");
    let fastest = console_cycles(&w, FREQUENCY_LOOP).0;
    assert!(
        (cheapest - fastest - LOOP_GAP).abs() <= 2.0,
        "the cheapest call's loop: CellGov {cheapest:.1}, the console {fastest:.1}",
    );
    for (l, &number) in LOOPS.iter().enumerate() {
        let (fastest, slowest) = console_cycles(&w, l);
        let cellgov = replay(&call_replay(l), 0).expect("the loop replays");
        assert!(
            (cellgov - fastest).abs() <= slowest - fastest,
            "syscall {number}, loop {l}: CellGov {cellgov:.1} cycles a call, the console \
             {fastest:.1} to {slowest:.1}",
        );
    }
}
