//! CellGov's LV2 round trips against the `ppu_lv2_call_refusals` console
//! capture: five loops of 4096 calls, five runs each. The kernel refuses
//! the calls of four of them for an object ID it never issued.
//!
//! Every loop runs the probe's `calls` body:
//!
//! 1. the two arguments from r28 and r29;
//! 2. zeroes in r5 to r10;
//! 3. the number from r27;
//! 4. the `sc`;
//! 5. the return-code check on cr4;
//! 6. the count in r31.
//!
//! Every refused call returns nonzero, so after the first call the
//! console skips the check's two instructions, as the replay does.

use crate::console_ppu_issue_timing::{replay, replay_r3, Replay, Seed};
use cellgov_ps3_abi::hw::ppu_issue::{
    ppu_syscall_outcome_round_trip_cycles, PPU_CYCLES_PER_TB_TICK, PPU_SYSCALL_ROUND_TRIP_CYCLES,
};
use cellgov_ps3_abi::lv2::errno::CELL_ESRCH;
use cellgov_ps3_abi::lv2::syscall as sc;

const CONSOLE: &str = "../../tests/micro/ppu_lv2_call_refusals/ps3/cech20-cex-493/observation.json";

/// The payload's word 60 when the probe wrote every word.
const DONE: u64 = 0x5050_4C52;

/// Calls a loop makes.
const CALLS: u64 = 1 << 12;

/// The object ID the probe names, which the kernel never issued.
const BAD_ID: u64 = 0x7FFF_FFF0;

/// The syscall each loop times, in loop order.
const LOOPS: [u64; 5] = [
    sc::TIME_GET_TIMEBASE_FREQUENCY,
    sc::SEMAPHORE_POST,
    sc::SEMAPHORE_TRY_WAIT,
    sc::EVENT_FLAG_CLEAR,
    sc::MUTEX_TRYLOCK,
];

/// The loop of [`LOOPS`] that times the cheapest call.
const FREQUENCY_LOOP: usize = 0;

/// The loop of [`LOOPS`] that repeats a loop of the `ppu_lv2_call_paths`
/// console capture.
const POST_LOOP: usize = 1;

/// The first loop of [`LOOPS`] whose call only this capture times on
/// its refusal.
const REFUSAL_LOOPS: usize = 2;

/// The calls whose refusal only this capture times.
pub(crate) const REFUSED: [u64; 3] = [
    sc::SEMAPHORE_TRY_WAIT,
    sc::EVENT_FLAG_CLEAR,
    sc::MUTEX_TRYLOCK,
];

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
    for l in POST_LOOP..LOOPS.len() {
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

/// The console's cycles a loop takes beside the cheapest call's trip.
fn loop_rest(w: &[u64]) -> u64 {
    console_cycles(w, FREQUENCY_LOOP).0.round() as u64 - PPU_SYSCALL_ROUND_TRIP_CYCLES
}

/// The probe's `calls` loop body, from its `mr r3,r28` to its `bne`.
const CALL_LOOP: [u32; 16] = [
    0x7f83_e378,
    0x7fa4_eb78,
    0x38a0_0000,
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

/// Each loop's seeds: its syscall number in r27, the bad ID in r28 for
/// an object call, and the count in r31.
const SEEDS: [[Seed; 3]; 5] = {
    let mut s = [[Seed::Counter(31); 3]; 5];
    let mut l = 0;
    while l < LOOPS.len() {
        s[l][0] = Seed::Gpr(27, LOOPS[l]);
        if l >= POST_LOOP {
            s[l][1] = Seed::Gpr(28, BAD_ID);
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
fn each_refusal_round_trip_is_its_fastest_run_less_the_loop_s_other_cycles() {
    let w = console_words();
    let rest = loop_rest(&w);
    assert_eq!(rest, 13, "the loop's own cycles");
    let esrch = u64::from(CELL_ESRCH.code);
    for (l, &number) in LOOPS.iter().enumerate().skip(REFUSAL_LOOPS) {
        assert_eq!(
            ppu_syscall_outcome_round_trip_cycles(number, esrch),
            console_cycles(&w, l).0.round() as u64 - rest,
            "the error round trip of syscall {number}, loop {l}",
        );
    }
    assert_eq!(&LOOPS[REFUSAL_LOOPS..], &REFUSED[..], "the refused calls");
}

#[test]
fn the_error_table_s_post_trip_lies_within_this_capture_s_post_spread() {
    // The post loop repeats a loop of the `ppu_lv2_call_paths` console
    // capture. That capture's fastest run sets the error table's entry.
    // This capture's runs of the same loop bound how far apart the two
    // captures sit.
    let w = console_words();
    let (fastest, slowest) = console_cycles(&w, POST_LOOP);
    let here = fastest.round() as u64 - loop_rest(&w);
    let tabled =
        ppu_syscall_outcome_round_trip_cycles(sc::SEMAPHORE_POST, u64::from(CELL_ESRCH.code));
    assert!(
        (here.abs_diff(tabled) as f64) <= slowest - fastest,
        "the table's {tabled} against this capture's {here}, a spread of {:.1}",
        slowest - fastest,
    );
}

#[test]
fn the_issue_model_times_each_refusal_loop_within_the_console_s_spread() {
    let w = console_words();
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

#[test]
fn cellgov_answers_each_refused_call_with_the_console_s_return_code() {
    let w = console_words();
    for (l, &number) in LOOPS.iter().enumerate().skip(POST_LOOP) {
        let r3 = replay_r3(&call_replay(l)).expect("the loop replays");
        assert_eq!(
            r3 & 0xFFFF_FFFF,
            w[32 + l],
            "syscall {number}, loop {l}: CellGov's return code against the console's",
        );
    }
}
