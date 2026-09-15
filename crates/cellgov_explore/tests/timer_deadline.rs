//! Whether a pending timer deadline is a dependency the relation sees.
//!
//! The guest clock is global. A unit parked on `sys_timer_usleep` wakes
//! at the first commit whose clock passed its deadline, so the ticks
//! other units spend before a store decide whether that store lands
//! before or after the sleeper's own. The relation still calls the
//! sleeper and those units independent, and the cover is complete
//! anyway. A timer wake publishes nothing. It changes which unit is
//! runnable, and every effect the woken unit then commits is an event
//! the relation already holds against the other writers. A DMA landing
//! differs (`shared_clock.rs`): it writes guest memory on no unit's
//! account, so no event carries it and the landing clause has to.

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
use cellgov_testkit::world::{CountingUnit, SleepingWriter, WritingUnit};
use cellgov_time::Budget;
use std::collections::BTreeSet;

const STEP_CAP: usize = 200;
const DEPTH: usize = 7;

/// Budget in ticks per step, against a one-microsecond sleep.
///
/// 1 usec is 1000 ticks, so the deadline falls three to four steps after
/// the park. That puts it inside the writer's run, which is what gives
/// the workload two outcomes.
const BUDGET: u64 = 300;

fn destination() -> ByteRange {
    ByteRange::new(GuestAddr::new(128), 4).unwrap()
}

fn workload() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(BUDGET), STEP_CAP);
    rt.register_unit_with(|id| SleepingWriter::new(id, 1, destination(), 0xbb));
    rt.register_unit_with(|id| WritingUnit::of_value(id, 3, destination(), 0xaa));
    rt.register_unit_with(|id| CountingUnit::new(id, 5));
    rt
}

/// `workload`'s shape where the two units that do not write differ in
/// what they spend.
///
/// A cheap counter and an expensive one touch no shared resource, so a
/// schedule that runs one before the other moves the sleeper's wake
/// without touching any range a footprint names.
fn skewed_workload() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(BUDGET), STEP_CAP);
    rt.register_unit_with(|id| SleepingWriter::new(id, 1, destination(), 0xbb));
    rt.register_unit_with(|id| WritingUnit::of_value(id, 2, destination(), 0xaa));
    rt.register_unit_with(|id| CountingUnit::of_cost(id, 3, 20));
    rt.register_unit_with(|id| CountingUnit::of_cost(id, 3, BUDGET));
    rt
}

/// The committed memory a schedule of `build` forced through `prefix`
/// reaches.
///
/// # Panics
///
/// Panics when a forced schedule stops short of a maximal execution.
fn run_prefix_of(build: fn() -> Runtime, prefix: &[UnitId]) -> u64 {
    let mut rt = build();
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

/// Units runnable once `build` runs `prefix`.
///
/// # Panics
///
/// Panics when a replay of `prefix` refuses a step or its commit.
fn runnable_after_of(build: fn() -> Runtime, prefix: &[UnitId]) -> Vec<UnitId> {
    let mut rt = build();
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

/// Every committed memory `build`'s choice tree reaches down to `depth`.
fn reachable_of(build: fn() -> Runtime, depth: usize) -> BTreeSet<u64> {
    fn walk(
        build: fn() -> Runtime,
        prefix: &mut Vec<UnitId>,
        depth: usize,
        seen: &mut BTreeSet<u64>,
    ) {
        let runnable = runnable_after_of(build, prefix);
        if depth == 0 || runnable.is_empty() {
            seen.insert(run_prefix_of(build, prefix));
            return;
        }
        for unit in runnable {
            prefix.push(unit);
            walk(build, prefix, depth - 1, seen);
            prefix.pop();
        }
    }
    let mut seen = BTreeSet::new();
    walk(build, &mut Vec::new(), depth, &mut seen);
    seen
}

/// Every committed memory the search answers for over `build`.
fn search_reach_of(build: fn() -> Runtime) -> (BTreeSet<u64>, bool) {
    let result = explore_window(
        build,
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
    (reached, result.bounds_hit)
}

#[test]
fn the_skewed_workload_is_schedule_sensitive() {
    assert_eq!(
        reachable_of(skewed_workload, DEPTH).len(),
        2,
        "a cheap step and an expensive one move the wake against the writer's last store",
    );
}

/// The relation does not see the tick dependency and does not need to;
/// the module doc says why.
#[test]
fn the_relation_calls_the_counter_and_the_sleeper_independent() {
    let mut rt = skewed_workload();
    let (log, stop) = observe_decisions(&mut rt);
    assert!(
        !stop.is_truncated(),
        "a prefix of the schedule answers for no pair: {stop}",
    );
    let execution = Execution::from_log(&log);
    assert!(
        execution.units_independent(UnitId::new(0), UnitId::new(2)),
        "the sleeper and the cheap counter share no resource the footprint names",
    );
}

#[test]
fn the_search_answers_for_the_skewed_workload_too() {
    let reachable = reachable_of(skewed_workload, 7);
    let (reached, bounds_hit) = search_reach_of(skewed_workload);
    assert!(!bounds_hit, "no bound stopped the search");
    // Both workloads withdraw their class count: a reversal names the
    // sleeper at a depth where it is still parked, and the drop is
    // counted. The cover is complete regardless, which is what
    // separates a withdrawn count from a missed outcome.
    assert_eq!(
        (
            explore_window(workload as fn() -> Runtime, &ExplorationConfig::default())
                .classes_explored,
            explore_window(
                skewed_workload as fn() -> Runtime,
                &ExplorationConfig::default()
            )
            .classes_explored,
        ),
        (None, None),
    );
    assert_eq!(
        reached, reachable,
        "the wake becomes observable only through the woken unit's own write, which the \
         relation holds against the other writer",
    );
}

#[test]
fn both_workloads_report_schedule_sensitive() {
    for build in [workload as fn() -> Runtime, skewed_workload] {
        let result = explore_window(build, &ExplorationConfig::default());
        assert_eq!(
            result.outcome,
            cellgov_explore::classify::OutcomeClass::ScheduleSensitive,
            "the wake moves against the writer's last store, so the memory is not stable",
        );
    }
}

#[test]
fn a_deadline_no_step_can_move_across_a_write_still_prunes() {
    // The sleeper writes a range nothing else touches, so its wake
    // position changes no committed memory; a blunt rule holding every
    // step against a pending deadline would still refuse to prune here.
    let build = || {
        let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(BUDGET), STEP_CAP);
        let elsewhere = ByteRange::new(GuestAddr::new(0), 4).unwrap();
        rt.register_unit_with(|id| SleepingWriter::new(id, 1, elsewhere, 0xbb));
        rt.register_unit_with(|id| WritingUnit::of_value(id, 2, destination(), 0xaa));
        rt.register_unit_with(|id| CountingUnit::of_cost(id, 3, 20));
        rt
    };
    let result = explore_window(build, &ExplorationConfig::default());
    assert_eq!(
        result.outcome,
        cellgov_explore::classify::OutcomeClass::ScheduleStable,
    );
    assert_eq!(
        result.classes_explored,
        Some(1),
        "no pair of these units conflicts, so one class covers the workload",
    );
}

#[test]
fn the_sleeper_wake_position_decides_committed_memory() {
    assert_eq!(
        reachable_of(workload, DEPTH).len(),
        2,
        "the order of the writer's last store against the sleeper's wake decides the outcome",
    );
}

#[test]
fn the_search_answers_for_every_reachable_memory() {
    let reachable = reachable_of(workload, DEPTH);
    let (reached, bounds_hit) = search_reach_of(workload);

    assert!(
        !bounds_hit,
        "no bound stopped the search, so what it reached is what it covers",
    );
    assert_eq!(
        reached, reachable,
        "the search answers for every committed memory a schedule can reach",
    );
}
