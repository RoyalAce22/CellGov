//! CellGov's LV2 round trips against the `ppu_lv2_call_paths` console
//! capture: thirteen loops of 4096 calls, five runs each, ten of them on
//! the error path an object ID the kernel never issued takes.
//!
//! Every loop runs the probe's `calls` body:
//!
//! 1. the four arguments from r26 to r29;
//! 2. zeroes in r7 to r10;
//! 3. the number from r25;
//! 4. the `sc`;
//! 5. the return code kept in r9;
//! 6. the return-code check on cr4;
//! 7. the count in r31.
//!
//! Every call but the page-attribute call returns nonzero, so after the
//! first call the console skips the check's two instructions, as the
//! replay does.

use crate::console_ppu_issue_timing::{replay, replay_r3, Replay, Seed};
use crate::console_ppu_lv2_call_refusals::REFUSED;
use cellgov_ps3_abi::hw::ppu_issue::{
    ppu_syscall_outcome_round_trip_cycles, ppu_syscall_round_trip_cycles, PPU_CYCLES_PER_TB_TICK,
    PPU_SYSCALL_ERROR_ROUND_TRIPS, PPU_SYSCALL_ROUND_TRIP_CYCLES,
};
use cellgov_ps3_abi::lv2::errno::{CELL_ENOSYS, CELL_ESRCH};
use cellgov_ps3_abi::lv2::syscall as sc;

const CONSOLE: &str = "../../tests/micro/ppu_lv2_call_paths/ps3/cech20-cex-493/observation.json";

/// The payload's word 60 when the probe wrote every word.
const DONE: u64 = 0x5050_4C50;

/// Calls a loop makes.
const CALLS: u64 = 1 << 12;

/// The object ID the probe names, which the kernel never issued.
const BAD_ID: u64 = 0x7FFF_FFF0;

/// The syscall each loop times, in loop order.
const LOOPS: [u64; 13] = [
    sc::TIME_GET_TIMEBASE_FREQUENCY,
    sc::PROCESS_GETPID,
    sc::MEMORY_GET_PAGE_ATTRIBUTE,
    sc::SEMAPHORE_POST,
    sc::SEMAPHORE_GET_VALUE,
    sc::MUTEX_LOCK,
    sc::MUTEX_UNLOCK,
    sc::EVENT_FLAG_GET,
    sc::EVENT_FLAG_SET,
    sc::EVENT_FLAG_TRY_WAIT,
    sc::EVENT_QUEUE_TRY_RECEIVE,
    sc::COND_SIGNAL,
    sc::EVENT_PORT_SEND,
];

/// The loop of [`LOOPS`] that times the cheapest call.
const FREQUENCY_LOOP: usize = 0;

/// The loop of [`LOOPS`] that times `sys_process_getpid`.
const GETPID_LOOP: usize = 1;

/// The loop of [`LOOPS`] that times `sys_memory_get_page_attribute`.
const PAGE_ATTRIBUTE_LOOP: usize = 2;

/// The first loop of [`LOOPS`] whose call names the bad ID.
const ERROR_LOOPS: usize = 3;

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
    assert_eq!(
        w[32 + PAGE_ATTRIBUTE_LOOP],
        0,
        "every page-attribute call returned 0"
    );
    assert_eq!(w[32 + GETPID_LOOP], w[48], "getpid returns the process ID");
    for l in ERROR_LOOPS..LOOPS.len() {
        assert_eq!(
            w[32 + l],
            u64::from(CELL_ESRCH.code),
            "loop {l} returns ESRCH",
        );
    }
    w
}

/// Loop `l`'s fastest and slowest runs, in core cycles a call.
fn console_cycles(w: &[u64], l: usize) -> (f64, f64) {
    let per = |ticks: u64| (ticks * PPU_CYCLES_PER_TB_TICK) as f64 / CALLS as f64;
    (per(w[2 * l]), per(w[2 * l + 1]))
}

/// The probe's `calls` loop body, from its `mr r3,r26` to its `bne`.
const CALL_LOOP: [u32; 17] = [
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
    0x7c69_1b78,
    0x4092_000c,
    0x787e_0020,
    0x2e3e_0000,
    0x2c3f_0001,
    0x3bff_ffff,
    0x4082_ffc0,
];

/// Each loop's seeds: its syscall number in r25, the bad ID in r26 for
/// an object call, and the count in r31.
const SEEDS: [[Seed; 3]; 13] = {
    let mut s = [[Seed::Counter(31); 3]; 13];
    let mut l = 0;
    while l < LOOPS.len() {
        s[l][0] = Seed::Gpr(25, LOOPS[l]);
        if l >= ERROR_LOOPS {
            s[l][1] = Seed::Gpr(26, BAD_ID);
        }
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
fn each_error_round_trip_is_its_fastest_run_less_the_loop_s_other_cycles() {
    let w = console_words();
    let rest = console_cycles(&w, FREQUENCY_LOOP).0.round() as u64 - PPU_SYSCALL_ROUND_TRIP_CYCLES;
    assert_eq!(rest, 15, "the loop's own cycles");
    let esrch = u64::from(CELL_ESRCH.code);
    for (l, &number) in LOOPS.iter().enumerate().skip(ERROR_LOOPS) {
        assert_eq!(
            ppu_syscall_outcome_round_trip_cycles(number, esrch),
            console_cycles(&w, l).0.round() as u64 - rest,
            "the error round trip of syscall {number}, loop {l}",
        );
    }
    assert_eq!(
        ppu_syscall_round_trip_cycles(sc::PROCESS_GETPID),
        console_cycles(&w, GETPID_LOOP).0.round() as u64 - rest,
        "the round trip of sys_process_getpid",
    );
    assert_eq!(
        ppu_syscall_round_trip_cycles(sc::MEMORY_GET_PAGE_ATTRIBUTE),
        console_cycles(&w, PAGE_ATTRIBUTE_LOOP).0.round() as u64 - rest,
        "the round trip of sys_memory_get_page_attribute",
    );
    let mut tabled: Vec<u64> = PPU_SYSCALL_ERROR_ROUND_TRIPS
        .iter()
        .map(|&(n, _)| n)
        .collect();
    // The `ppu_lv2_call_refusals` console capture times the rest.
    let mut looped: Vec<u64> = [&LOOPS[ERROR_LOOPS..], &REFUSED[..]].concat();
    tabled.sort_unstable();
    looped.sort_unstable();
    assert_eq!(
        tabled, looped,
        "the error table holds every error loop of both captures once"
    );
}

#[test]
fn an_error_return_takes_the_error_trip_and_a_success_the_success_trip() {
    for &(number, error) in &PPU_SYSCALL_ERROR_ROUND_TRIPS {
        assert_eq!(
            ppu_syscall_outcome_round_trip_cycles(number, u64::from(CELL_ESRCH.code)),
            error,
            "syscall {number}",
        );
        assert_eq!(
            ppu_syscall_outcome_round_trip_cycles(number, 0),
            ppu_syscall_round_trip_cycles(number),
            "syscall {number}",
        );
    }
    // An error from a call with no error entry keeps its success trip.
    assert_eq!(
        ppu_syscall_outcome_round_trip_cycles(sc::PROCESS_GETPID, u64::from(CELL_ESRCH.code)),
        ppu_syscall_round_trip_cycles(sc::PROCESS_GETPID),
    );
}

#[test]
fn the_issue_model_times_each_paths_loop_within_the_console_s_spread() {
    let w = console_words();
    // The model's cycles beside the call are the same in every loop, so
    // each loop lands as far from its fastest run as the cheapest call's
    // loop does. The spread alone does not tell `sys_event_flag_get`'s
    // 5775-cycle error trip from its 5889-cycle success trip.
    let gap = replay(&call_replay(FREQUENCY_LOOP), 0).expect("the loop replays")
        - console_cycles(&w, FREQUENCY_LOOP).0;
    for (l, &number) in LOOPS.iter().enumerate() {
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

#[test]
fn cellgov_answers_each_error_loop_s_call_with_the_console_s_return_code() {
    let w = console_words();
    for (l, &number) in LOOPS.iter().enumerate().skip(ERROR_LOOPS) {
        let r3 = replay_r3(&call_replay(l)).expect("the loop replays");
        assert_eq!(
            r3 & 0xFFFF_FFFF,
            w[32 + l],
            "syscall {number}, loop {l}: CellGov's return code against the console's",
        );
    }
}

#[test]
fn cellgov_refuses_the_page_attribute_call_the_console_serves() {
    // The console answers 0. CellGov does not serve the call and answers
    // ENOSYS, so its replay skips the return-code check's two
    // instructions that the console runs on every call.
    let w = console_words();
    assert_eq!(w[32 + PAGE_ATTRIBUTE_LOOP], 0, "the console serves it");
    let r3 = replay_r3(&call_replay(PAGE_ATTRIBUTE_LOOP)).expect("the loop replays");
    assert_eq!(
        r3 & 0xFFFF_FFFF,
        u64::from(CELL_ENOSYS.code),
        "CellGov's answer changed: hold it to the console's 0",
    );
}
