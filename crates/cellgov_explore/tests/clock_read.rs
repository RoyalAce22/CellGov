//! Whether a step that reads the guest clock is a dependency the
//! relation sees.
//!
//! A unit that reads the time base and stores it commits a value every
//! other unit's tick spend decided. The range it writes is the same
//! whatever that value is, so a footprint naming ranges records the
//! store and not what made it differ.
//!
//! This is the third reader of the one clock, after the DMA landing
//! `shared_clock.rs` covers and the timer deadline `timer_deadline.rs`
//! covers.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_explore::config::ExplorationConfig;
use cellgov_explore::execution::Execution;
use cellgov_explore::explorer::explore_window;
use cellgov_explore::observer::observe_decisions;
use cellgov_explore::prescribed::PrescribedScheduler;
use cellgov_explore::util::run_to_stall;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_testkit::world::{ClockWriter, CountingUnit};
use cellgov_time::Budget;
use std::collections::{BTreeMap, BTreeSet};

const STEP_CAP: usize = 200;
const DEPTH: usize = 6;
const BUDGET: u64 = 64;
/// Ticks the cheap counter spends per step; the expensive one spends
/// the whole budget. The two make the outcome set below.
const CHEAP_COST: u64 = 8;

const READER: UnitId = UnitId::new(0);
const CHEAP: UnitId = UnitId::new(1);
const EXPENSIVE: UnitId = UnitId::new(2);

fn destination() -> ByteRange {
    ByteRange::new(GuestAddr::new(128), 8).unwrap()
}

/// One clock reader against two counters that spend different ticks.
///
/// The counters touch no shared resource, so the relation calls them
/// independent of each other. The clock clause alone holds either of
/// them against the reader, and which of them runs first decides the
/// tick count the reader stores.
fn workload() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(BUDGET), STEP_CAP);
    let reader = rt.register_unit_with(|id| ClockWriter::new(id, destination()));
    let cheap = rt.register_unit_with(|id| CountingUnit::of_cost(id, 2, CHEAP_COST));
    let expensive = rt.register_unit_with(|id| CountingUnit::of_cost(id, 2, BUDGET));
    // Every case below names units by id, so a registration inserted
    // above would retarget them.
    assert_eq!(reader, READER, "registration order moved the reader");
    assert_eq!(cheap, CHEAP, "registration order moved the cheap counter");
    assert_eq!(
        expensive, EXPENSIVE,
        "registration order moved the expensive counter",
    );
    rt
}

/// The committed memory hash and the tick count a schedule forced
/// through `prefix` leaves stored.
///
/// # Panics
///
/// Panics when a forced schedule stops short of a maximal execution.
fn run_prefix(prefix: &[UnitId]) -> (u64, u64) {
    let mut rt = workload();
    rt.set_scheduler(PrescribedScheduler::new(
        prefix.iter().copied().map(Some).collect(),
    ));
    let stop = run_to_stall(&mut rt, STEP_CAP);
    assert!(
        !stop.is_truncated(),
        "the schedule through {prefix:?} stopped short: {stop}",
    );
    (rt.committed_memory_hash(), stored_ticks(&rt))
}

/// The little-endian tick count the reader left at `destination`.
///
/// # Panics
///
/// Panics when the destination is unreadable, which no schedule of this
/// workload produces.
fn stored_ticks(rt: &Runtime) -> u64 {
    let bytes = rt
        .memory()
        .read(destination())
        .expect("the destination is mapped for every schedule");
    u64::from_le_bytes(bytes.try_into().expect("the destination is eight bytes"))
}

/// Units runnable once `prefix` runs.
///
/// # Panics
///
/// Panics when replaying `prefix` refuses a step or its commit.
fn runnable_after(prefix: &[UnitId]) -> Vec<UnitId> {
    let mut rt = workload();
    rt.set_scheduler(PrescribedScheduler::new(
        prefix.iter().copied().map(Some).collect(),
    ));
    for _ in 0..prefix.len() {
        let step = rt
            .step()
            .expect("the walk built this prefix from runnable units");
        rt.commit_step(&step.result, &step.effects)
            .expect("no step of this workload refuses its commit");
    }
    rt.registry().runnable_ids().collect()
}

/// Every outcome the choice tree reaches, as committed hash to the tick
/// count that made it.
fn every_reachable_memory() -> BTreeMap<u64, u64> {
    fn walk(prefix: &mut Vec<UnitId>, depth: usize, seen: &mut BTreeMap<u64, u64>) {
        let runnable = runnable_after(prefix);
        if depth == 0 || runnable.is_empty() {
            let (hash, ticks) = run_prefix(prefix);
            seen.insert(hash, ticks);
            return;
        }
        for unit in runnable {
            prefix.push(unit);
            walk(prefix, depth - 1, seen);
            prefix.pop();
        }
    }
    let mut seen = BTreeMap::new();
    walk(&mut Vec::new(), DEPTH, &mut seen);
    seen
}

/// Every committed memory the search answers for, beside the cover it
/// claims and whether a bound stopped it.
fn memories_the_search_reaches() -> (BTreeSet<u64>, Option<usize>, bool) {
    let result = explore_window(
        workload,
        &ExplorationConfig {
            max_schedules: 100_000,
            max_steps_per_run: 10_000,
        },
    );
    let mut reached = BTreeSet::new();
    if !result.baseline_stop.is_truncated() {
        reached.insert(result.baseline_hash);
    }
    for record in &result.schedules {
        if !record.truncated {
            reached.insert(record.memory_hash);
        }
    }
    (reached, result.classes_explored, result.bounds_hit)
}

/// Nine outcomes: the reader stores `8a + 64b` for `a` cheap steps and
/// `b` expensive ones run before it, and each counter runs two.
#[test]
fn the_reader_commits_one_memory_per_tick_count_it_can_see() {
    let reachable = every_reachable_memory();
    let ticks: BTreeSet<u64> = reachable.values().copied().collect();
    assert_eq!(
        ticks,
        BTreeSet::from([0, 8, 16, 64, 72, 80, 128, 136, 144]),
        "every count of cheap and expensive steps before the reader is a distinct outcome",
    );
    assert_eq!(
        reachable.len(),
        ticks.len(),
        "each tick count leaves its own committed memory",
    );
}

#[test]
fn the_relation_holds_the_reader_against_the_counters() {
    let mut rt = workload();
    let (log, stop) = observe_decisions(&mut rt);
    assert!(!stop.is_truncated(), "a prefix answers for no pair: {stop}");
    let execution = Execution::from_log(&log);
    assert!(
        !execution.units_independent(READER, CHEAP),
        "the cheap counter's ticks decide what the reader stores",
    );
    assert!(
        !execution.units_independent(READER, EXPENSIVE),
        "and so do the expensive one's",
    );
    // Without this the two above pass under a relation that pairs every
    // step with every other.
    assert!(
        execution.units_independent(CHEAP, EXPENSIVE),
        "neither counter reads the clock, and they share nothing else",
    );
}

#[test]
fn the_verdict_reads_schedule_sensitive() {
    let result = explore_window(workload, &ExplorationConfig::default());
    assert_eq!(
        result.outcome,
        cellgov_explore::classify::OutcomeClass::ScheduleSensitive,
        "the stored tick count moves with the schedule, so the memory is not stable",
    );
}

/// The clause pairs on the read itself: no footprint tracks the value
/// from the register that received it to a store.
#[test]
fn a_reader_whose_store_nothing_else_reaches_still_conflicts() {
    let build = || {
        let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(BUDGET), STEP_CAP);
        // Writes its own range, which nothing else reads or writes.
        let private = ByteRange::new(GuestAddr::new(0), 8).unwrap();
        let reader = rt.register_unit_with(|id| ClockWriter::new(id, private));
        let cheap = rt.register_unit_with(|id| CountingUnit::of_cost(id, 2, CHEAP_COST));
        assert_eq!(reader, READER, "registration order moved the reader");
        assert_eq!(cheap, CHEAP, "registration order moved the counter");
        rt
    };
    let mut rt = build();
    let (log, stop) = observe_decisions(&mut rt);
    assert!(!stop.is_truncated(), "{stop}");
    let execution = Execution::from_log(&log);
    assert!(
        !execution.units_independent(READER, CHEAP),
        "the counter cannot reach what the reader stored, and the pair still conflicts",
    );
}

/// The count a class holds one committed memory, so nine outcomes need
/// nine classes and the run has to claim exactly that.
#[test]
fn the_search_reaches_every_outcome_and_claims_one_class_per_outcome() {
    let reachable = every_reachable_memory();
    let (reached, classes, bounds_hit) = memories_the_search_reaches();
    assert!(!bounds_hit, "no bound stopped the search");

    let missed: BTreeSet<u64> = reachable
        .iter()
        .filter(|(hash, _)| !reached.contains(hash))
        .map(|(_, ticks)| *ticks)
        .collect();
    assert!(
        missed.is_empty(),
        "outcomes the search does not reach, by the tick count that makes each: {missed:?}",
    );
    assert_eq!(
        classes,
        Some(9),
        "one execution per class, and no class left over",
    );
}
