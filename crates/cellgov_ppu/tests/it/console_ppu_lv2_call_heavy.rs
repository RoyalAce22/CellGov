//! CellGov's LV2 round trips against the `ppu_lv2_call_heavy` console
//! capture: each PRX module, thread and memory call timed alone, 32
//! rounds.
//!
//! Each call sits between two `mftb` reads, after other kernel work. The
//! cheapest call's samples in the same bracket give the cycles the
//! bracket adds beside its round trip.

use cellgov_ps3_abi::hw::ppu_issue::{
    ppu_syscall_round_trip_cycles, PPU_CYCLES_PER_TB_TICK, PPU_SYSCALL_ROUND_TRIPS,
    PPU_SYSCALL_ROUND_TRIP_CYCLES,
};
use cellgov_ps3_abi::lv2::syscall as sc;

const CONSOLE: &str = "../../tests/micro/ppu_lv2_call_heavy/ps3/cech20-cex-493/observation.json";

/// The payload's word 60 when the probe wrote every word.
const DONE: u64 = 0x5050_4C48;

/// Rounds the probe makes.
const ROUNDS: usize = 32;

/// The syscall of each call the probe times, in payload order. Call 0 is
/// the cheapest call. Calls 2 and 3 are the two start phases. Calls 4 and
/// 5 are the two stop phases.
const CALLS: [u64; 12] = [
    sc::TIME_GET_TIMEBASE_FREQUENCY,
    sc::SYS_PRX_LOAD_MODULE,
    sc::SYS_PRX_START_MODULE,
    sc::SYS_PRX_START_MODULE,
    sc::SYS_PRX_STOP_MODULE,
    sc::SYS_PRX_STOP_MODULE,
    sc::SYS_PRX_UNLOAD_MODULE,
    sc::PPU_THREAD_CREATE,
    sc::PPU_THREAD_START,
    sc::PPU_THREAD_JOIN,
    sc::MEMORY_ALLOCATE,
    sc::MEMORY_FREE,
];

/// The calls this capture adds to the table.
pub(crate) const LOOPS: [u64; 9] = [
    sc::SYS_PRX_LOAD_MODULE,
    sc::SYS_PRX_START_MODULE,
    sc::SYS_PRX_STOP_MODULE,
    sc::SYS_PRX_UNLOAD_MODULE,
    sc::PPU_THREAD_CREATE,
    sc::PPU_THREAD_START,
    sc::PPU_THREAD_JOIN,
    sc::MEMORY_ALLOCATE,
    sc::MEMORY_FREE,
];

/// The time-base frequency the cheapest call returns.
const TIMEBASE_HZ: u64 = 79_800_000;

/// The value each thread passes to its exit.
const EXIT_VALUE: u64 = 0x5157;

/// The console's payload: its 64 big-endian words, then each call's
/// samples in time-base ticks.
fn console_payload() -> (Vec<u64>, Vec<Vec<u64>>) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 2048);
    let w: Vec<u64> = p[..512]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(w[60], DONE, "the probe wrote every word");
    assert_eq!(w[58], ROUNDS as u64, "rounds");
    assert_eq!(w[59], CALLS.len() as u64, "calls a round");
    assert_eq!(w[0], TIMEBASE_HZ, "the cheapest call returns the frequency");
    for (c, &word) in w.iter().enumerate().take(CALLS.len()).skip(1) {
        assert_eq!(word, 0, "every call {c} returned 0, or an id for the load");
    }
    assert_eq!(w[16], ROUNDS as u64, "every load returned an id");
    assert_eq!(w[19], ROUNDS as u64, "every thread ran");
    assert_eq!(w[20], EXIT_VALUE, "the join returned the exit value");
    let samples = p[512..]
        .chunks_exact(4 * ROUNDS)
        .map(|call| {
            call.chunks_exact(4)
                .map(|s| u64::from(u32::from_be_bytes(s.try_into().expect("4 bytes"))))
                .collect()
        })
        .collect();
    (w, samples)
}

/// Each call's lower median sample, in core cycles.
fn medians(samples: &[Vec<u64>]) -> Vec<u64> {
    samples
        .iter()
        .map(|s| {
            let mut s = s.clone();
            s.sort_unstable();
            s[(s.len() - 1) / 2] * PPU_CYCLES_PER_TB_TICK
        })
        .collect()
}

#[test]
fn each_call_s_round_trip_is_its_median_sample_less_the_bracket_s_other_cycles() {
    let (_, samples) = console_payload();
    let median = medians(&samples);
    let rest = median[0] - PPU_SYSCALL_ROUND_TRIP_CYCLES;
    assert_eq!(rest, 2511, "the bracket's own cycles");
    let trip = |c: usize| median[c] - rest;
    // A start and a stop each make one call of each phase.
    let expected = [
        (sc::SYS_PRX_LOAD_MODULE, trip(1)),
        (sc::SYS_PRX_START_MODULE, (trip(2) + trip(3)) / 2),
        (sc::SYS_PRX_STOP_MODULE, (trip(4) + trip(5)) / 2),
        (sc::SYS_PRX_UNLOAD_MODULE, trip(6)),
        (sc::PPU_THREAD_CREATE, trip(7)),
        (sc::PPU_THREAD_START, trip(8)),
        (sc::PPU_THREAD_JOIN, trip(9)),
        (sc::MEMORY_ALLOCATE, trip(10)),
        (sc::MEMORY_FREE, trip(11)),
    ];
    for (number, cycles) in expected {
        assert_eq!(
            ppu_syscall_round_trip_cycles(number),
            cycles,
            "the round trip of syscall {number}",
        );
    }
}

#[test]
fn a_module_load_costs_over_ten_times_every_other_timed_call() {
    let load = ppu_syscall_round_trip_cycles(sc::SYS_PRX_LOAD_MODULE);
    for &(number, _) in PPU_SYSCALL_ROUND_TRIPS
        .iter()
        .filter(|&&(n, _)| n != sc::SYS_PRX_LOAD_MODULE)
    {
        let other = ppu_syscall_round_trip_cycles(number);
        assert!(
            load > 10 * other,
            "the load {load} against syscall {number}'s {other}",
        );
    }
}
