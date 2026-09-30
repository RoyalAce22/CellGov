//! A warp that wakes two units is an ordinary branching point.
//!
//! Where no unit is runnable the runtime warps guest time and fires
//! everything due at the tick it lands on. Two waits that expire at the
//! same tick therefore wake together. Which of the two runs first is a
//! choice, and the search owes an execution for it.
//!
//! The deadlines meet by arithmetic. A wait's deadline is the tick its
//! syscall committed at plus its timeout, and one step costs the
//! thousand ticks a microsecond of timeout converts to (`BUDGET`). So
//! the unit that parks one step later asks for one microsecond less,
//! and the two deadlines meet.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use std::cell::Cell;

use cellgov_core::Runtime;
use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_explore::{explore_optimal, ExplorationConfig};
use cellgov_lv2::PpuThreadAttrs;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::{Budget, GuestTicks, InstructionCost};

/// One step costs this many ticks, which is also what one microsecond
/// of timeout converts to.
const BUDGET: u64 = 1_000;
const STEP_CAP: usize = 200;
const FLAG_ID: u32 = 1;
/// A null result pointer, which the kernel skips.
///
/// The expiry then writes no guest memory. A write there would make the
/// warp step a clock reader, which conflicts with every step, and the
/// parked waits below it would ask for reversals no state reaches.
const RESULT_PTR: u32 = 0;
/// The word both units write when they wake, so the committed memory
/// shows the waking order.
const SHARED: u64 = 0x100;

const FIRST: UnitId = UnitId::new(0);
const SECOND: UnitId = UnitId::new(1);

/// Waits on a flag whose bits never arrive, then writes `mark`; the wait
/// ends in the warp, so the write is this unit's first step after it.
#[derive(Clone)]
struct Waker {
    id: UnitId,
    timeout_usec: u64,
    mark: u8,
    steps: Cell<u64>,
}

impl ExecutionUnit for Waker {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= 2 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        if n == 1 {
            let mut args = [0u64; 9];
            args[0] = cellgov_ps3_abi::lv2::syscall::EVENT_FLAG_WAIT;
            args[1] = u64::from(FLAG_ID);
            args[2] = 0b10;
            args[3] = 0x01;
            args[4] = u64::from(RESULT_PTR);
            args[5] = self.timeout_usec;
            return ExecutionStepResult {
                yield_reason: YieldReason::Syscall,
                consumed_cost: InstructionCost::new(budget.raw()),
                local_diagnostics: LocalDiagnostics::with_pc(0x1000),
                fault: None,
                syscall_args: Some(args),
            };
        }
        effects.push(Effect::shared_write(
            ByteRange::new(GuestAddr::new(SHARED), 4).unwrap(),
            WritePayload::new(vec![self.mark; 4]),
            self.id,
            GuestTicks::ZERO,
        ));
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

/// Two waits that land on one tick, so one warp wakes both.
fn two_waits_one_tick() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(BUDGET), STEP_CAP);
    let first = rt.register_unit_with(|id| Waker {
        id,
        timeout_usec: 2,
        mark: 0xA1,
        steps: Cell::new(0),
    });
    let second = rt.register_unit_with(|id| Waker {
        id,
        timeout_usec: 1,
        mark: 0xB2,
        steps: Cell::new(0),
    });
    assert_eq!(first, FIRST, "registration order moved the first waiter");
    assert_eq!(second, SECOND, "registration order moved the second waiter");

    let attrs = PpuThreadAttrs {
        entry: 0,
        arg: 0,
        stack_base: 0,
        stack_size: 0,
        priority: 0,
        tls_base: 0,
    };
    rt.lv2_host_mut()
        .seed_primary_ppu_thread(first, attrs.clone());
    rt.lv2_host_mut()
        .ppu_threads_mut()
        .create(second, attrs)
        .expect("a second thread id");
    rt.lv2_host_mut()
        .event_flags_mut()
        .create_with_id(FLAG_ID, 0)
        .expect("a fresh event-flag id");
    rt
}

#[test]
fn one_warp_wakes_both_waiters() {
    let mut rt = two_waits_one_tick();
    let mut warps = 0usize;
    for _ in 0..16 {
        let idle = rt.registry().runnable_ids().next().is_none();
        let Ok(step) = rt.step() else { break };
        if idle {
            warps += 1;
            assert_eq!(
                rt.last_runnable().len(),
                2,
                "the warp woke {:?}, so the deadlines did not meet",
                rt.last_runnable(),
            );
        }
        rt.commit_step(&step.result, &step.effects)
            .expect("no step of this workload refuses its commit");
    }
    assert_eq!(warps, 1, "one warp, and it is the branching point");
}

#[test]
fn the_search_runs_one_execution_per_waking_order() {
    let result = explore_optimal(
        two_waits_one_tick,
        &ExplorationConfig {
            max_schedules: 100,
            max_steps_per_run: 1_000,
        },
    );

    assert!(
        !result.baseline_stop.is_truncated(),
        "the baseline reaches the end: {}",
        result.baseline_stop,
    );
    assert!(!result.bounds_hit, "no bound stopped this search");
    assert_eq!(
        result
            .schedules
            .iter()
            .map(|record| (record.branch_step, record.alternate_choice))
            .collect::<Vec<_>>(),
        vec![(2, SECOND)],
        "the alternate re-decides at the warp depth, with the unit the \
         baseline did not run there",
    );
    assert_eq!(
        result.reversals_dropped, 0,
        "the warp depth delivered the reversal it was asked for",
    );
    assert_eq!(
        result.classes_explored,
        Some(2),
        "one class per waking order, and the count is the claim",
    );
    assert_eq!(
        result.schedules.len(),
        1,
        "the baseline plus one alternate is both orders",
    );
}
