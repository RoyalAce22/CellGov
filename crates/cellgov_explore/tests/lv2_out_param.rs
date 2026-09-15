//! Whether a write an LV2 handler commits reaches the relation.
//!
//! The runtime applies a handler's effects at dispatch rather than
//! through the commit pipeline, so the calling unit's own step carries
//! an empty effect list. A relation that reads the effect list alone
//! sees a step that touched nothing. It then calls that step
//! independent of a unit that writes the very bytes the handler landed.
//! That is a false independence.
//!
//! `sys_time_get_current_time` is the smallest arm that shows it. It
//! writes both out parameters itself and derives them from the dispatch
//! tick, so the bytes it lands differ per schedule.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use std::cell::Cell;

use cellgov_core::Runtime;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_explore::config::ExplorationConfig;
use cellgov_explore::execution::Execution;
use cellgov_explore::explorer::explore_window;
use cellgov_explore::observer::observe_decisions;
use cellgov_explore::prescribed::PrescribedScheduler;
use cellgov_explore::util::run_to_stall;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_testkit::world::WritingUnit;
use cellgov_time::{Budget, InstructionCost};
use std::collections::BTreeSet;

const STEP_CAP: usize = 200;
const BUDGET: u64 = 16;

const SEC_OUT: u64 = 128;
const NSEC_OUT: u64 = 136;
/// Nothing the caller or the peer touches, so a pair over it states
/// that the recording is range-precise.
const ELSEWHERE: u64 = 64;
/// The byte the peer lays over the handler's output.
const PEER_BYTE: u8 = 0xAA;

const CALLER: UnitId = UnitId::new(0);
const PEER: UnitId = UnitId::new(1);
const LONER: UnitId = UnitId::new(2);

fn nsec_range() -> ByteRange {
    ByteRange::new(GuestAddr::new(NSEC_OUT), 8).unwrap()
}

fn elsewhere_range() -> ByteRange {
    ByteRange::new(GuestAddr::new(ELSEWHERE), 8).unwrap()
}

/// Calls `sys_time_get_current_time` once, then finishes.
///
/// It emits no effect of its own: everything this step lands on guest
/// memory, the handler lands.
#[derive(Clone)]
struct TimeCaller {
    id: UnitId,
    steps: Cell<u64>,
}

impl ExecutionUnit for TimeCaller {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= 1 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.steps.set(self.steps.get() + 1);
        let mut args = [0u64; 9];
        args[0] = cellgov_ps3_abi::lv2::syscall::TIME_GET_CURRENT_TIME;
        args[1] = SEC_OUT;
        args[2] = NSEC_OUT;
        ExecutionStepResult {
            yield_reason: YieldReason::Syscall,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::with_pc(0x1000),
            fault: None,
            syscall_args: Some(args),
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

/// The caller against a unit that writes the same bytes, plus one that
/// writes bytes nobody else touches.
fn workload() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(BUDGET), STEP_CAP);
    let caller = rt.register_unit_with(|id| TimeCaller {
        id,
        steps: Cell::new(0),
    });
    let peer = rt.register_unit_with(|id| WritingUnit::of_value(id, 1, nsec_range(), PEER_BYTE));
    let loner = rt.register_unit_with(|id| WritingUnit::of_value(id, 1, elsewhere_range(), 0x11));
    // Every case below names units by id, so a registration inserted
    // above would retarget them.
    assert_eq!(caller, CALLER, "registration order moved the caller");
    assert_eq!(peer, PEER, "registration order moved the peer");
    assert_eq!(loner, LONER, "registration order moved the loner");
    rt
}

#[test]
fn the_handler_lands_bytes_the_callers_own_step_never_names() {
    let mut rt = workload();
    rt.set_scheduler(PrescribedScheduler::new(vec![Some(CALLER)]));
    let before = rt.memory().read(nsec_range()).unwrap().to_vec();

    let step = rt.step().expect("the caller is runnable");
    assert_eq!(step.unit, CALLER);
    assert!(
        step.effects.is_empty(),
        "the syscall step emits nothing itself: {:?}",
        step.effects,
    );
    rt.commit_step(&step.result, &step.effects)
        .expect("the dispatch commits");

    let after = rt.memory().read(nsec_range()).unwrap().to_vec();
    assert_ne!(
        before, after,
        "the handler wrote the out parameter during a step that named no write",
    );
    assert!(
        !rt.last_lv2_effects().is_empty(),
        "and the runtime publishes what it applied",
    );
}

#[test]
fn the_relation_holds_the_caller_against_the_unit_that_shares_the_bytes() {
    let mut rt = workload();
    let (log, stop) = observe_decisions(&mut rt);
    assert!(!stop.is_truncated(), "a prefix answers for no pair: {stop}");
    let execution = Execution::from_log(&log);

    assert!(
        !execution.units_independent(CALLER, PEER),
        "the peer writes the bytes the handler wrote, so the order decides them",
    );
    assert!(
        !execution.units_independent(CALLER, LONER),
        "and the loner's ticks decide the value the handler stamped, \
         though it touches none of those bytes",
    );
    // Without this the two above pass under a relation that pairs every
    // step with every other.
    assert!(
        execution.units_independent(PEER, LONER),
        "neither writer reads the clock, and their ranges are disjoint",
    );
}

/// The committed memory a schedule forced through `prefix` leaves.
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

/// Every committed memory the choice tree reaches.
fn every_reachable_memory() -> BTreeSet<u64> {
    fn walk(prefix: &mut Vec<UnitId>, seen: &mut BTreeSet<u64>) {
        let runnable = runnable_after(prefix);
        if runnable.is_empty() {
            seen.insert(run_prefix(prefix));
            return;
        }
        for unit in runnable {
            prefix.push(unit);
            walk(prefix, seen);
            prefix.pop();
        }
    }
    let mut seen = BTreeSet::new();
    walk(&mut Vec::new(), &mut seen);
    seen
}

/// The caller's position decides the tick the handler stamps, and the
/// peer's decides whether that stamp survives.
#[test]
fn the_out_parameter_gives_the_workload_more_than_one_outcome() {
    assert!(
        every_reachable_memory().len() > 1,
        "a workload with one outcome cannot show a missed class",
    );
}

#[test]
fn the_search_answers_for_every_memory_the_workload_reaches() {
    let reachable = every_reachable_memory();
    let result = explore_window(workload, &ExplorationConfig::default());
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
        "committed memories the search never reaches: {missed:?}",
    );
    assert_eq!(
        reachable.len(),
        3,
        "the stamp at each of two tick counts, and the peer laying over it",
    );
    assert_eq!(
        result.classes_explored,
        Some(4),
        "one more class than there are outcomes: the clock clause pairs the \
         caller with the loner, which splits a class whose two orders leave \
         the same memory. A false dependency costs cover, not soundness",
    );
}
