//! What a step that rides a landing costs the relation and the race
//! scan.
//!
//! Such a step conflicts with every step, so it is the shape a title
//! window is most likely to hold and the one the quadratic scan is
//! measured on. `benches/explore_bench.rs` times the same two shapes;
//! these pins are the part `cargo test` runs.

use super::*;
use cellgov_mem::{ByteRange, GuestAddr};

const UNITS: u64 = 4;
const EVENTS: usize = 256;

fn word(slot: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(slot * 8), 4).expect("a 4-byte word")
}

/// `EVENTS` steps round-robin over `UNITS` units, each unit writing its
/// own word. With `rider` a step in the middle also carries a transfer
/// in flight over the word it writes.
fn round_robin_writers(rider: bool) -> Execution {
    let mut execution = Execution::new();
    let rider_at = EVENTS / 2;
    for index in 0..EVENTS {
        let unit = index as u64 % UNITS;
        let mut footprint = StepFootprint {
            shared_writes: vec![word(unit)],
            ..StepFootprint::default()
        };
        if rider && index == rider_at {
            footprint.inflight_dma_ranges.push(word(unit));
        }
        execution.push(UnitId::new(unit), footprint);
    }
    execution
}

/// The premise: without the rider no pair conflicts, so every scan runs
/// to the start.
#[test]
fn without_a_rider_the_scan_never_stops_short() {
    let execution = round_robin_writers(false);
    let hb = execution.happens_before();
    assert_eq!(hb.cost().joins, 0);
    assert_eq!(
        hb.cost().conflict_tests,
        RIDERLESS_CONFLICT_TESTS,
        "each event scans every earlier event of every other unit",
    );
    assert!(execution.races(&hb).is_empty());
}

/// The rider conflicts with every step, so every later event's scan
/// stops at it and the relation orders every earlier event before it:
/// the relation gets cheaper, and the races are the rider's alone.
#[test]
fn a_rider_cuts_the_scan_short_and_races_with_its_neighbours() {
    let execution = round_robin_writers(true);
    let hb = execution.happens_before();
    assert!(
        hb.cost().conflict_tests < RIDERLESS_CONFLICT_TESTS,
        "a conflict the clock carries costs no test: {} tests",
        hb.cost().conflict_tests,
    );
    assert_eq!(hb.cost().conflict_tests, RIDER_CONFLICT_TESTS);
    assert_eq!(hb.cost().joins, RIDER_JOINS);
    let races = execution.races(&hb);
    assert_eq!(races.len(), RIDER_RACES);
    let rider = EVENTS / 2;
    assert!(
        races
            .iter()
            .all(|race| race.first.index == rider || race.second.index == rider),
        "every race names the rider: {races:?}",
    );
}

/// The figures are this crate's own, pinned as regression witnesses.
/// A move is a change to the scan to explain, not a number to
/// re-bless.
const RIDERLESS_CONFLICT_TESTS: usize = 24_576;
const RIDER_CONFLICT_TESTS: usize = 12_198;
const RIDER_JOINS: usize = 6;
const RIDER_RACES: usize = 6;
