//! An SPU read of `SPU_RdDec` is a clock read the schedule explorer
//! sees.
//!
//! The SPU looks at its decrementer in one step, which anchors the
//! count, and reads it in the next. The count it stores into local
//! store is the time base that passed between the two, so it depends on
//! which other units' steps ran in between. Two counter units spend
//! that time, and they share no state with the SPU.

// [CBEA p:139 s:9.7] the SPU decrementer runs at the time-base frequency.
// [CBEA p:140 s:9.7.2] SPU_RdDec returns the current count.

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_explore::config::ExplorationConfig;
use cellgov_explore::execution::Execution;
use cellgov_explore::explorer::explore_window;
use cellgov_explore::observer::observe_decisions;
use cellgov_explore::prescribed::PrescribedScheduler;
use cellgov_explore::util::run_to_stall;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::SPU_RD_DEC;
use cellgov_spu::SpuExecutionUnit;
use cellgov_testkit::world::CountingUnit;
use cellgov_time::Budget;
use std::collections::BTreeSet;

const STEP_CAP: usize = 64;
const DEPTH: usize = 8;
/// Instructions per step: the SPU's first step runs this many nops.
const BUDGET: u64 = 4096;
/// Ticks each counter spends in its one step. Both move the time base
/// by more than one tick, so each changes the count the SPU stores.
const CHEAP_COST: u64 = 1000;
const EXPENSIVE_COST: u64 = BUDGET;

const READER: UnitId = UnitId::new(0);
const CHEAP: UnitId = UnitId::new(1);
const EXPENSIVE: UnitId = UnitId::new(2);

const NOP: u32 = 0x4020_0000;
const STOP: u32 = 0;

/// `rdch rt, $ch<channel>`.
const fn rdch(channel: u8, rt: u32) -> u32 {
    (0x00D << 21) | ((channel as u32) << 7) | rt
}

/// `stqd rt, 16*i10(ra)`.
const fn stqd(rt: u32, i10: u32, ra: u32) -> u32 {
    (0x24 << 24) | ((i10 & 0x3FF) << 14) | (ra << 7) | rt
}

/// An SPU that runs one step of nops, then stores the count at
/// local-store address 16 and stops.
fn reader(id: UnitId) -> SpuExecutionUnit {
    let mut spu = SpuExecutionUnit::new(id);
    let state = spu.state_mut();
    let words =
        std::iter::repeat_n(NOP, BUDGET as usize).chain([rdch(SPU_RD_DEC, 2), stqd(2, 1, 0), STOP]);
    for (i, word) in words.enumerate() {
        state.ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    spu
}

fn workload() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(BUDGET), STEP_CAP);
    let spu = rt.register_unit_with(reader);
    let cheap = rt.register_unit_with(|id| CountingUnit::of_cost(id, 1, CHEAP_COST));
    let expensive = rt.register_unit_with(|id| CountingUnit::of_cost(id, 1, EXPENSIVE_COST));
    assert_eq!(spu, READER, "registration order moved the reader");
    assert_eq!(cheap, CHEAP, "registration order moved the cheap counter");
    assert_eq!(
        expensive, EXPENSIVE,
        "registration order moved the expensive counter"
    );
    rt
}

/// The observable hash a schedule forced through `prefix` leaves.
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
        "the schedule through {prefix:?} stopped short: {stop}"
    );
    rt.observable_hash()
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

/// Every observable the choice tree reaches.
fn every_reachable_observable() -> BTreeSet<u64> {
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

#[test]
fn the_count_the_reader_stores_moves_with_the_counters_between_its_steps() {
    assert!(
        every_reachable_observable().len() > 1,
        "the schedule decides the count, so more than one observable is reachable",
    );
}

#[test]
fn the_relation_holds_the_reader_against_the_counters() {
    let mut rt = workload();
    let (log, stop) = observe_decisions(&mut rt);
    assert!(!stop.is_truncated(), "{stop}");
    let execution = Execution::from_log(&log);
    assert!(
        !execution.units_independent(READER, CHEAP),
        "the cheap counter's ticks decide the count the reader stores",
    );
    assert!(
        !execution.units_independent(READER, EXPENSIVE),
        "and so do the expensive one's",
    );
    // Without this assertion, the two above also pass under a relation
    // that pairs every step with every other.
    assert!(
        execution.units_independent(CHEAP, EXPENSIVE),
        "neither counter reads the clock, and they share nothing else",
    );
}

#[test]
fn the_search_reaches_every_count_the_reader_can_store() {
    let reachable = every_reachable_observable();
    let result = explore_window(
        workload,
        &ExplorationConfig {
            max_schedules: 100_000,
            max_steps_per_run: 10_000,
        },
    );
    assert!(!result.bounds_hit, "no bound stopped the search");
    let mut reached = BTreeSet::new();
    if !result.baseline_stop.is_truncated() {
        reached.insert(result.baseline_hash);
    }
    for record in &result.schedules {
        if !record.truncated {
            reached.insert(record.memory_hash);
        }
    }
    let missed: Vec<u64> = reachable.difference(&reached).copied().collect();
    assert!(
        missed.is_empty(),
        "observables the search does not reach: {missed:x?}"
    );
}
