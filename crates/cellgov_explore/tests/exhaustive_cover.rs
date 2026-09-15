//! The search's cover claim against every schedule of a small workload.
//!
//! `shared_clock.rs` holds the search against one hand-written
//! schedule. This walks the choice tree of `shared_clock`'s shape with a
//! shorter counter: a transfer in flight, a writer over its destination,
//! and a unit that only spends ticks. Two committed memories is the
//! total: the destination is the only range any schedule writes twice,
//! and it ends holding either the transfer's bytes or the writer's last
//! store.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_explore::config::ExplorationConfig;
use cellgov_explore::explorer::explore_window;
use cellgov_explore::prescribed::PrescribedScheduler;
use cellgov_explore::util::run_to_stall;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_testkit::world::{CountingUnit, DmaSubmitter, WritingUnit};
use cellgov_time::Budget;
use std::collections::BTreeSet;

const STEP_CAP: usize = 200;
/// Choices the walk forces before it lets a prefix run itself out.
///
/// The workload retires thirteen steps -- two, three and eight -- so
/// this is a prefix of the choice tree. Both committed memories sit
/// inside it.
const DEPTH: usize = 8;

/// The latency the two outcomes rest on.
///
/// Budget 2 and ten ticks put the landing inside the writer's run. A
/// change to either leaves one outcome, and every case below then reads
/// a different workload.
const LATENCY: u64 = 10;

fn destination() -> ByteRange {
    ByteRange::new(GuestAddr::new(128), 4).unwrap()
}

fn workload() -> Runtime {
    assert_eq!(
        cellgov_core::DEFAULT_DMA_LATENCY_TICKS.raw(),
        LATENCY,
        "the workspace latency is what puts the landing between the writer's stores",
    );
    let src = ByteRange::new(GuestAddr::new(0), 4).unwrap();
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(2), STEP_CAP);
    rt.register_unit_with(|id| {
        DmaSubmitter::new(id, src, destination(), vec![0xde, 0xad, 0xbe, 0xef])
    });
    rt.register_unit_with(|id| WritingUnit::new(id, 3, destination()));
    rt.register_unit_with(|id| CountingUnit::new(id, 8));
    rt
}

/// The committed memory a schedule forced through `prefix` reaches.
///
/// # Panics
///
/// Panics when a forced schedule stops short of a maximal execution.
fn run_prefix(prefix: &[UnitId]) -> u64 {
    let mut rt = workload();
    rt.set_scheduler(PrescribedScheduler::new(
        prefix.iter().copied().map(Some).collect(),
    ));
    let stop = run_to_stall(&mut rt, STEP_CAP);
    assert!(
        !stop.is_truncated(),
        "the schedule through {prefix:?} stopped short: {stop}",
    );
    rt.committed_memory_hash()
}

/// Units runnable once `prefix` runs.
///
/// # Panics
///
/// Panics when replaying `prefix` refuses a step or its commit. Every
/// prefix the walk builds was runnable where the walk built it.
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

/// Every committed memory the choice tree reaches down to `DEPTH`,
/// each branch then run out under the round-robin fallback.
fn every_reachable_memory() -> BTreeSet<u64> {
    fn walk(prefix: &mut Vec<UnitId>, depth: usize, seen: &mut BTreeSet<u64>) {
        let runnable = runnable_after(prefix);
        if depth == 0 || runnable.is_empty() {
            seen.insert(run_prefix(prefix));
            return;
        }
        for unit in runnable {
            prefix.push(unit);
            walk(prefix, depth - 1, seen);
            prefix.pop();
        }
    }
    let mut seen = BTreeSet::new();
    walk(&mut Vec::new(), DEPTH, &mut seen);
    seen
}

/// Every committed memory the search answers for, beside the cover it
/// claims, the reversals it dropped, and whether a bound stopped it.
fn memories_the_search_reaches() -> (BTreeSet<u64>, Option<usize>, usize, bool) {
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
    (
        reached,
        result.classes_explored,
        result.reversals_dropped,
        result.bounds_hit,
    )
}

#[test]
fn two_committed_memories_are_reachable() {
    assert_eq!(
        every_reachable_memory().len(),
        2,
        "the order of the writer's last store against the landing decides the outcome",
    );
}

#[test]
fn the_search_answers_for_every_reachable_memory() {
    let reachable = every_reachable_memory();
    let (reached, classes, dropped, bounds_hit) = memories_the_search_reaches();

    assert!(
        !bounds_hit,
        "no bound stopped the search, so what it reached is what it covers",
    );
    assert_eq!(
        reached, reachable,
        "the search answers for every committed memory a schedule can reach",
    );
    assert_eq!(
        reached.len(),
        2,
        "and the workload really does reach two, so this is not vacuous",
    );
    assert_eq!(
        classes, None,
        "a dropped reversal withdraws the class count for the whole run",
    );
    // The run gives up hundreds of branches and still answers for every
    // outcome; an empty class count alone reads as one. The figure counts
    // once per lost sequence per frame, so the search's own race handling
    // moves it.
    assert_eq!(
        dropped, 290,
        "the branches this workload's depths could not deliver",
    );
}
