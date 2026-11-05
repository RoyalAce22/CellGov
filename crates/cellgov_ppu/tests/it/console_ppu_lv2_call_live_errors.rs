//! CellGov's LV2 round trips against the `ppu_lv2_call_live_errors`
//! console capture: eight loops of 4096 calls, five runs each, seven of
//! them on a semaphore, mutex or event flag the call finds and then
//! refuses.
//!
//! The probe's `calls` body passes four arguments from r26 to r29 and the
//! number from r25, as the `ppu_lv2_call_paths` probe's does. The replay
//! names no live object, so the tests hold these trips against the table
//! alone. The `ppu_lv2_call_cost` tests hold that console capture's
//! success trips the same way.

use cellgov_ps3_abi::hw::ppu_issue::{
    ppu_syscall_outcome_round_trip_cycles, PPU_CYCLES_PER_TB_TICK,
    PPU_SYSCALL_LIVE_ERROR_ROUND_TRIPS, PPU_SYSCALL_ROUND_TRIP_CYCLES,
};
use cellgov_ps3_abi::lv2::errno::{self, Lv2ErrCode};
use cellgov_ps3_abi::lv2::syscall as sc;

const CONSOLE: &str =
    "../../tests/micro/ppu_lv2_call_live_errors/ps3/cech20-cex-493/observation.json";

/// The payload's word 60 when the probe wrote every word.
const DONE: u64 = 0x5050_4C45;

/// Calls a loop makes.
const CALLS: u64 = 1 << 12;

/// The loop that times the cheapest call.
const FREQUENCY_LOOP: usize = 0;

/// Each refusing loop's syscall and the error its state calls for, in
/// loop order from loop 1:
///
/// 1. a semaphore at count 0;
/// 2. a semaphore at its maximum;
/// 3. a post of a negative count;
/// 4. a lock of a non-recursive mutex the caller holds;
/// 5. a trylock of that mutex;
/// 6. an unlock of that mutex, which the caller holds no longer;
/// 7. an event flag whose value matches no bit.
const REFUSALS: [(u64, Lv2ErrCode); 7] = [
    (sc::SEMAPHORE_TRY_WAIT, errno::CELL_EBUSY),
    (sc::SEMAPHORE_POST, errno::CELL_EBUSY),
    (sc::SEMAPHORE_POST, errno::CELL_EINVAL),
    (sc::MUTEX_LOCK, errno::CELL_EDEADLK),
    (sc::MUTEX_TRYLOCK, errno::CELL_EDEADLK),
    (sc::MUTEX_UNLOCK, errno::CELL_EPERM),
    (sc::EVENT_FLAG_TRY_WAIT, errno::CELL_EBUSY),
];

/// The console's payload as big-endian words, with every setup and
/// teardown step checked.
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
    for (i, &rc) in w[48..57].iter().enumerate() {
        assert_eq!(rc, 0, "setup or teardown word {}", 48 + i);
    }
    for (i, &(number, code)) in REFUSALS.iter().enumerate() {
        let l = i + 1;
        assert_eq!(
            w[32 + l],
            u64::from(code.code),
            "syscall {number}, loop {l} returns {}",
            code.symbol,
        );
    }
    w
}

/// Loop `l`'s fastest run, in core cycles a call.
fn console_fastest(w: &[u64], l: usize) -> u64 {
    ((w[2 * l] * PPU_CYCLES_PER_TB_TICK) as f64 / CALLS as f64).round() as u64
}

#[test]
fn each_live_error_round_trip_is_its_fastest_run_less_the_loop_s_other_cycles() {
    let w = console_words();
    let rest = console_fastest(&w, FREQUENCY_LOOP) - PPU_SYSCALL_ROUND_TRIP_CYCLES;
    assert_eq!(rest, 20, "the loop's own cycles");
    for (i, &(number, _)) in REFUSALS.iter().enumerate() {
        let l = i + 1;
        assert_eq!(
            ppu_syscall_outcome_round_trip_cycles(number, w[32 + l]),
            console_fastest(&w, l) - rest,
            "the round trip of syscall {number} returning {:#x}, loop {l}",
            w[32 + l],
        );
    }
}

#[test]
fn the_live_error_table_holds_each_refusing_loop_once() {
    let mut tabled: Vec<(u64, u32)> = PPU_SYSCALL_LIVE_ERROR_ROUND_TRIPS
        .iter()
        .map(|&(n, code, _)| (n, code))
        .collect();
    let mut looped: Vec<(u64, u32)> = REFUSALS.iter().map(|&(n, c)| (n, c.code)).collect();
    tabled.sort_unstable();
    looped.sort_unstable();
    assert_eq!(tabled, looped);
}

#[test]
fn a_live_error_is_charged_in_r3_s_sign_extended_form_too() {
    // The kernel sign-extends the code into r3; the table matches its low
    // 32 bits.
    let w = console_words();
    let rest = console_fastest(&w, FREQUENCY_LOOP) - PPU_SYSCALL_ROUND_TRIP_CYCLES;
    for (i, &(number, code)) in REFUSALS.iter().enumerate() {
        let l = i + 1;
        assert_eq!(
            ppu_syscall_outcome_round_trip_cycles(number, u64::from(code)),
            console_fastest(&w, l) - rest,
            "syscall {number}, {}, loop {l}",
            code.symbol,
        );
    }
}
