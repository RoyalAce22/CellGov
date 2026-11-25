//! CellGov's LV2 round trips against the `ppu_lv2_call_untimed` console
//! capture: nine loops of 4096 uncontended calls, five runs each. The
//! first loop times the cheapest call. The other eight time calls that no
//! earlier capture times.
//!
//! Every loop runs the `calls` body of `ppu_lv2_call_cost`, word for
//! word. The cheapest call's loop takes 970 cycles a call here and 942
//! there, so the entries from each capture exclude that capture's own
//! loop cycles.

use crate::console_ppu_issue_timing::{replay, replay_r3, Replay, Seed, ONE_LINE};
use crate::console_ppu_lv2_call_cost::CALL_LOOP;
use cellgov_ps3_abi::hw::ppu_issue::{
    ppu_syscall_round_trip_cycles, PPU_CYCLES_PER_TB_TICK, PPU_SYSCALL_ROUND_TRIP_CYCLES,
};
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::syscall as sc;

const CONSOLE: &str = "../../tests/micro/ppu_lv2_call_untimed/ps3/cech20-cex-493/observation.json";

/// The payload's word 60 when the probe wrote every word.
const DONE: u64 = 0x5050_4C55;

/// Calls a loop makes.
const CALLS: u64 = 1 << 12;

/// The syscall each loop times, in loop order.
pub(crate) const LOOPS: [u64; 9] = [
    sc::TIME_GET_TIMEBASE_FREQUENCY,
    sc::SEMAPHORE_WAIT,
    sc::EVENT_FLAG_WAIT,
    sc::EVENT_FLAG_TRY_WAIT,
    sc::COND_SIGNAL,
    sc::COND_SIGNAL_ALL,
    sc::PROCESS_GETPPID,
    sc::PROCESS_GET_SDK_VERSION,
    sc::TIME_GET_TIMEZONE,
];

/// The loop of [`LOOPS`] that times the cheapest call.
const FREQUENCY_LOOP: usize = 0;

/// The loop of `sys_process_getppid`, which returns the parent's ID.
const GETPPID_LOOP: usize = 6;

/// The loop of `sys_time_get_timezone`.
const TIMEZONE_LOOP: usize = 8;

/// The time-base frequency the cheapest call returns.
const TIMEBASE_HZ: u64 = 79_800_000;

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
        "condition variable create",
        "semaphore destroy",
        "event flag destroy",
        "condition variable destroy",
        "mutex destroy",
    ]) {
        assert_eq!(w[word], 0, "{what}");
    }
    assert_eq!(
        w[32 + FREQUENCY_LOOP],
        TIMEBASE_HZ,
        "loop 0 returns the frequency"
    );
    // The word keeps the loop's first nonzero r3, so a refused call would
    // leave its error code there as a nonzero word too.
    let ppid = w[32 + GETPPID_LOOP];
    assert!(
        ppid != 0 && errno::ENTRIES.iter().all(|e| u64::from(e.code) != ppid),
        "getppid returns the parent's ID, not an error: {ppid:#x}",
    );
    for l in (1..LOOPS.len()).filter(|&l| l != GETPPID_LOOP) {
        assert_eq!(w[32 + l], 0, "every call of loop {l} returned 0");
    }
    assert_eq!(w[56], 1, "the event flag's bits the waits read");
    w
}

/// Loop `l`'s fastest and slowest runs, in core cycles a call.
fn console_cycles(w: &[u64], l: usize) -> (f64, f64) {
    let per = |ticks: u64| (ticks * PPU_CYCLES_PER_TB_TICK) as f64 / CALLS as f64;
    (per(w[2 * l]), per(w[2 * l + 1]))
}

/// Each loop's seeds: its syscall number in r25 and the count in r31.
/// The time zone call gets its two result pointers in r26 and r27, as
/// the probe passes them.
const SEEDS: [[Seed; 4]; 9] = {
    let mut s = [[Seed::Counter(31); 4]; 9];
    let mut l = 0;
    while l < LOOPS.len() {
        s[l][0] = Seed::Gpr(25, LOOPS[l]);
        l += 1;
    }
    s[TIMEZONE_LOOP][1] = Seed::Gpr(26, ONE_LINE);
    s[TIMEZONE_LOOP][2] = Seed::Gpr(27, ONE_LINE + 8);
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
fn each_call_s_round_trip_is_its_fastest_run_less_the_loop_s_other_cycles() {
    let w = console_words();
    let rest = console_cycles(&w, FREQUENCY_LOOP).0.round() as u64 - PPU_SYSCALL_ROUND_TRIP_CYCLES;
    assert_eq!(rest, 41, "the loop's own cycles");
    for (l, &number) in LOOPS.iter().enumerate() {
        assert_eq!(
            ppu_syscall_round_trip_cycles(number),
            console_cycles(&w, l).0.round() as u64 - rest,
            "the round trip of syscall {number}, loop {l}",
        );
    }
}

#[test]
fn the_issue_model_times_each_query_loop_as_far_from_the_console_as_the_cheapest_call_s() {
    // Only the queries replay on the console's path: the object calls'
    // r26 names no object, so CellGov refuses them.
    let w = console_words();
    let gap = replay(&call_replay(FREQUENCY_LOOP), 0).expect("the loop replays")
        - console_cycles(&w, FREQUENCY_LOOP).0;
    for l in [GETPPID_LOOP, TIMEZONE_LOOP] {
        let (fastest, slowest) = console_cycles(&w, l);
        let cellgov = replay(&call_replay(l), 0).expect("the loop replays");
        assert!(
            (cellgov - fastest - gap).abs() <= 2.0,
            "syscall {}, loop {l}: CellGov {cellgov:.1} cycles a call, the console \
             {fastest:.1} to {slowest:.1}; the cheapest call's loop is {gap:.1} from it",
            LOOPS[l],
        );
    }
    let r3 = replay_r3(&call_replay(TIMEZONE_LOOP)).expect("the loop replays");
    assert_eq!(
        r3, 0,
        "CellGov serves the time zone call as the console does"
    );
}
