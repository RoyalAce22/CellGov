//! Whether a pending timer deadline is a dependency the relation sees.
//!
//! Each unit keeps its own clock. A unit parked on `sys_timer_usleep`
//! wakes once no runnable unit's clock is below its deadline, so a unit
//! whose clock lags holds the wake back. A store another unit makes past
//! the deadline lands before the sleeper's own when it runs ahead of the
//! lagging unit, and after it otherwise.
//!
//! The relation still calls the sleeper and the lagging unit
//! independent, and the cover is complete anyway. A timer wake publishes
//! nothing. It changes which unit is runnable, and every effect the
//! woken unit then commits is an event the relation already holds
//! against the other writers. A DMA landing differs (`shared_clock.rs`):
//! it writes guest memory on no unit's account, so no event carries it
//! and the landing clause has to.

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
/// 1 usec is 3192 ticks. The sleeper parks at its tick 958, so the
/// deadline is tick 4150, inside the writer's fifth step (ticks 3832 to
/// 4790). The writer's sixth store starts past it, which is what gives
/// the workload two outcomes.
const BUDGET: u64 = 958;
/// Stores the writer makes; the last starts at tick 4790.
const WRITER_STEPS: u64 = 6;
/// A cheap counter's step, about a fifteenth of the budget.
const CHEAP: u64 = 64;

const SLEEPER: UnitId = UnitId::new(0);
const WRITER: UnitId = UnitId::new(1);
const COUNTER: UnitId = UnitId::new(2);

fn destination() -> ByteRange {
    ByteRange::new(GuestAddr::new(128), 4).unwrap()
}

fn workload() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(BUDGET), STEP_CAP);
    let sleeper = rt.register_unit_with(|id| SleepingWriter::new(id, 1, destination(), 0xbb));
    let writer =
        rt.register_unit_with(|id| WritingUnit::of_value(id, WRITER_STEPS, destination(), 0xaa));
    let counter = rt.register_unit_with(|id| CountingUnit::new(id, 5));
    assert_eq!(sleeper, SLEEPER, "registration order moved the sleeper");
    assert_eq!(writer, WRITER, "registration order moved the writer");
    assert_eq!(counter, COUNTER, "registration order moved the counter");
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
        rt.commit_unchecked(&step.result, &step.effects)
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
fn a_store_run_ahead_of_the_lagging_counter_lands_under_the_sleeper() {
    let baseline = run_prefix_of(workload, &[]);
    // The sleeper parks first, so its deadline is pending under every
    // store. Without it every store runs before the park, and the
    // sleeper writes last whatever the counter's clock does.
    let mut prefix = vec![SLEEPER];
    prefix.extend([WRITER; WRITER_STEPS as usize]);
    let writer_first = run_prefix_of(workload, &prefix);
    assert_ne!(
        baseline, writer_first,
        "the counter's clock holds the wake back past a store run ahead of it",
    );
}

/// The relation does not see the tick dependency and does not need to;
/// the module doc says why.
#[test]
fn the_relation_calls_the_counter_and_the_sleeper_independent() {
    let mut rt = workload();
    let (log, stop) = observe_decisions(&mut rt);
    assert!(
        !stop.is_truncated(),
        "a prefix of the schedule answers for no pair: {stop}",
    );
    let execution = Execution::from_log(&log);
    assert!(
        execution.units_independent(SLEEPER, COUNTER),
        "the sleeper and the counter share no resource the footprint names",
    );
}

#[test]
fn the_workload_reports_schedule_sensitive() {
    let result = explore_window(workload, &ExplorationConfig::default());
    assert_eq!(
        result.outcome,
        cellgov_explore::classify::OutcomeClass::ScheduleSensitive,
        "the wake moves against the writer's last store, so the memory is not stable",
    );
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
        rt.register_unit_with(|id| CountingUnit::of_cost(id, 3, CHEAP));
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
        "the wake becomes observable only through the woken unit's own write, which the \
         relation holds against the other writer",
    );
}
