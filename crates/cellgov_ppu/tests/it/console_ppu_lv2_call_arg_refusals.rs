//! CellGov's LV2 round trips against the `ppu_lv2_call_arg_refusals`
//! console capture: seven loops of 4096 calls, five runs each, five of
//! them refused for a null result pointer and one a mutex trylock while
//! another thread holds the mutex.
//!
//! The probe's `calls` body passes the number in r27 and two arguments in
//! r28 and r29; the compiler folds its other two arguments to zero. The
//! replay names no live object, so the tests hold the object calls'
//! trips against the table alone. The time and memory-size loops replay
//! through CellGov.

use crate::console_ppu_issue_timing::{replay, replay_r3, Replay, Seed};
use cellgov_ps3_abi::hw::ppu_issue::{
    ppu_syscall_outcome_round_trip_cycles, PPU_CYCLES_PER_TB_TICK, PPU_SYSCALL_REFUSAL_ROUND_TRIPS,
    PPU_SYSCALL_ROUND_TRIP_CYCLES,
};
use cellgov_ps3_abi::lv2::errno::{self, Lv2ErrCode};
use cellgov_ps3_abi::lv2::syscall as sc;

const CONSOLE: &str =
    "../../tests/micro/ppu_lv2_call_arg_refusals/ps3/cech20-cex-493/observation.json";

/// The payload's word 60 when the probe wrote every word.
const DONE: u64 = 0x5050_4C41;

/// Calls a loop makes.
const CALLS: u64 = 1 << 12;

/// The loop that times the cheapest call.
const FREQUENCY_LOOP: usize = 0;

/// Each refusing loop's syscall and the error the console gave, in loop
/// order from loop 1:
///
/// - loops 1 to 5: the time, memory-size, semaphore-value, event-flag
///   and queue calls with a null result pointer;
/// - loop 6: a trylock of a mutex another thread holds.
const REFUSALS: [(u64, Lv2ErrCode); 6] = [
    (sc::TIME_GET_CURRENT_TIME, errno::CELL_EFAULT),
    (sc::MEMORY_GET_USER_MEMORY_SIZE, errno::CELL_EFAULT),
    (sc::SEMAPHORE_GET_VALUE, errno::CELL_EFAULT),
    (sc::EVENT_FLAG_GET, errno::CELL_EFAULT),
    (sc::EVENT_QUEUE_TRY_RECEIVE, errno::CELL_EFAULT),
    (sc::MUTEX_TRYLOCK, errno::CELL_EBUSY),
];

/// The loops that name no kernel object, which the replay runs through
/// CellGov's dispatch.
const OBJECTLESS_LOOPS: [usize; 2] = [1, 2];

/// The setup and teardown words, from the first semaphore's create to its
/// destroy.
const STEP_WORDS: std::ops::RangeInclusive<usize> = 32..=48;

/// The console's payload as big-endian words, with every setup and
/// teardown step and every loop's return code checked.
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
    assert_eq!(w[57], 0, "every setup and teardown step returned 0");
    for i in STEP_WORDS {
        assert_eq!(w[i], 0, "setup or teardown word {i}");
    }
    for (i, &(number, code)) in REFUSALS.iter().enumerate() {
        let l = i + 1;
        for (word, which) in [(16 + l, "first"), (24 + l, "last")] {
            assert_eq!(
                w[word],
                u64::from(code.code),
                "syscall {number}, loop {l}: the {which} call returns {}",
                code.symbol,
            );
        }
    }
    w
}

/// Loop `l`'s fastest and slowest runs, in core cycles a call.
fn console_cycles(w: &[u64], l: usize) -> (f64, f64) {
    let per = |ticks: u64| (ticks * PPU_CYCLES_PER_TB_TICK) as f64 / CALLS as f64;
    (per(w[2 * l]), per(w[2 * l + 1]))
}

/// The cycles of the probe's loop beside the cheapest call's round trip.
fn loop_rest(w: &[u64]) -> u64 {
    console_cycles(w, FREQUENCY_LOOP).0.round() as u64 - PPU_SYSCALL_ROUND_TRIP_CYCLES
}

/// The probe's `calls` loop body, from its `mr r3,r28` to its `bne`.
const CALL_LOOP: [u32; 16] = [
    0x7f83_e378,
    0x3880_0000,
    0x7fa5_eb78,
    0x38c0_0000,
    0x38e0_0000,
    0x3900_0000,
    0x3920_0000,
    0x3940_0000,
    0x7f6b_db78,
    0x4400_0002,
    0x4092_000c,
    0x787e_0020,
    0x2e3e_0000,
    0x2c3f_0001,
    0x3bff_ffff,
    0x4082_ffc4,
];

/// The seeds of the frequency, time and memory-size loops: the number in
/// r27 and the count in r31. Their arguments stay zero, as the probe
/// passes them.
const SEEDS: [[Seed; 2]; 3] = [
    [
        Seed::Gpr(27, sc::TIME_GET_TIMEBASE_FREQUENCY),
        Seed::Counter(31),
    ],
    [Seed::Gpr(27, sc::TIME_GET_CURRENT_TIME), Seed::Counter(31)],
    [
        Seed::Gpr(27, sc::MEMORY_GET_USER_MEMORY_SIZE),
        Seed::Counter(31),
    ],
];

/// The replay of loop `l`, one of the frequency loop and
/// [`OBJECTLESS_LOOPS`].
fn call_replay(l: usize) -> Replay {
    Replay {
        loop_index: l,
        words: &CALL_LOOP,
        seeds: &SEEDS[l],
    }
}

#[test]
fn each_refusal_round_trip_is_its_fastest_run_less_the_loop_s_other_cycles() {
    let w = console_words();
    let rest = loop_rest(&w);
    assert_eq!(rest, 17, "the loop's own cycles");
    for (i, &(number, code)) in REFUSALS.iter().enumerate() {
        let l = i + 1;
        assert_eq!(
            ppu_syscall_outcome_round_trip_cycles(number, u64::from(code.code)),
            console_cycles(&w, l).0.round() as u64 - rest,
            "the round trip of syscall {number} returning {}, loop {l}",
            code.symbol,
        );
    }
}

#[test]
fn the_refusal_table_holds_each_refusing_loop_once() {
    let mut tabled: Vec<(u64, u32)> = PPU_SYSCALL_REFUSAL_ROUND_TRIPS
        .iter()
        .map(|&(n, code, _)| (n, code))
        .collect();
    let mut looped: Vec<(u64, u32)> = REFUSALS.iter().map(|&(n, c)| (n, c.code)).collect();
    tabled.sort_unstable();
    looped.sort_unstable();
    assert_eq!(tabled, looped);
}

#[test]
fn cellgov_refuses_each_objectless_loop_s_null_pointer_with_the_console_s_code() {
    let w = console_words();
    for l in OBJECTLESS_LOOPS {
        let r3 = replay_r3(&call_replay(l)).expect("the loop replays");
        assert_eq!(
            r3 & 0xFFFF_FFFF,
            w[16 + l],
            "syscall {}, loop {l}: CellGov's return code against the console's",
            REFUSALS[l - 1].0,
        );
    }
}

#[test]
fn the_issue_model_times_each_objectless_refusal_loop_at_the_console_s_fastest_run() {
    let w = console_words();
    // The model's cycles beside the call are the same in every loop, so
    // each loop lands as far from its fastest run as the cheapest call's
    // loop does.
    let gap = replay(&call_replay(FREQUENCY_LOOP), 0).expect("the loop replays")
        - console_cycles(&w, FREQUENCY_LOOP).0;
    for l in OBJECTLESS_LOOPS {
        let number = REFUSALS[l - 1].0;
        let (fastest, slowest) = console_cycles(&w, l);
        let cellgov = replay(&call_replay(l), 0).expect("the loop replays");
        assert!(
            (cellgov - fastest).abs() <= slowest - fastest,
            "syscall {number}, loop {l}: CellGov {cellgov:.1} cycles a call, the console \
             {fastest:.1} to {slowest:.1}",
        );
        assert!(
            (cellgov - fastest - gap).abs() <= 2.0,
            "syscall {number}, loop {l}: CellGov {cellgov:.1} cycles a call, {:.1} from \
             the console's fastest {fastest:.1}; the cheapest call's loop is {gap:.1} from it",
            cellgov - fastest,
        );
    }
}
