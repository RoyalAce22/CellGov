//! CellGov's LV2 round trips against the `ppu_lv2_call_lifecycle`
//! console capture: each synchronization object class created and
//! destroyed 128 times a run, five runs each.
//!
//! A create loop and a destroy loop have different bodies, so the probe
//! times the cheapest call through each. A create's entry excludes the
//! create loop's own cycles, and a destroy's entry excludes the destroy
//! loop's own cycles.

use cellgov_ps3_abi::hw::ppu_issue::{
    ppu_syscall_round_trip_cycles, PPU_CYCLES_PER_TB_TICK, PPU_SYSCALL_ROUND_TRIP_CYCLES,
};
use cellgov_ps3_abi::lv2::syscall as sc;

const CONSOLE: &str =
    "../../tests/micro/ppu_lv2_call_lifecycle/ps3/cech20-cex-493/observation.json";

/// The payload's word 60 when the probe wrote every word.
const DONE: u64 = 0x5050_4C55;

/// Objects a run creates and destroys.
const OBJECTS: u64 = 128;

/// The loops that time the cheapest call through the create body and the
/// destroy body.
const BASELINES: [usize; 2] = [0, 1];

/// The syscall each loop from loop 2 times, a create and then its destroy.
pub(crate) const LOOPS: [u64; 10] = [
    sc::LWMUTEX_CREATE,
    sc::LWMUTEX_DESTROY,
    sc::MUTEX_CREATE,
    sc::MUTEX_DESTROY,
    sc::SEMAPHORE_CREATE,
    sc::SEMAPHORE_DESTROY,
    sc::EVENT_FLAG_CREATE,
    sc::EVENT_FLAG_DESTROY,
    sc::COND_CREATE,
    sc::COND_DESTROY,
];

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
    assert_eq!(w[58], OBJECTS, "objects a run");
    assert_eq!(w[48], 0, "the shared mutex's create");
    assert_eq!(w[49], 0, "the shared mutex's destroy");
    for l in BASELINES {
        assert_eq!(w[32 + l], TIMEBASE_HZ, "loop {l} returns the frequency");
    }
    for l in 2..2 + LOOPS.len() {
        assert_eq!(w[32 + l], 0, "every call of loop {l} returned 0");
    }
    w
}

/// Loop `l`'s fastest run, in core cycles a call, to the nearest cycle.
fn console_fastest(w: &[u64], l: usize) -> u64 {
    ((w[2 * l] * PPU_CYCLES_PER_TB_TICK) as f64 / OBJECTS as f64).round() as u64
}

#[test]
fn each_create_and_destroy_round_trip_is_its_fastest_run_less_its_loop_s_other_cycles() {
    let w = console_words();
    let rest = BASELINES.map(|l| console_fastest(&w, l) - PPU_SYSCALL_ROUND_TRIP_CYCLES);
    assert_eq!(rest, [18, 15], "the create and destroy loops' own cycles");
    for (i, &number) in LOOPS.iter().enumerate() {
        assert_eq!(
            ppu_syscall_round_trip_cycles(number),
            console_fastest(&w, 2 + i) - rest[i % 2],
            "the round trip of syscall {number}, loop {}",
            2 + i,
        );
    }
}

#[test]
fn a_create_costs_about_two_to_three_times_its_destroy() {
    for pair in LOOPS.chunks_exact(2) {
        let ratio = ppu_syscall_round_trip_cycles(pair[0]) as f64
            / ppu_syscall_round_trip_cycles(pair[1]) as f64;
        assert!(
            (1.8..2.9).contains(&ratio),
            "syscall {} against {}: {ratio:.2}",
            pair[0],
            pair[1],
        );
    }
}
