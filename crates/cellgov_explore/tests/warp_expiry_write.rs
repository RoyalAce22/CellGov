//! Whether a guest write the time warp lands reaches the relation.
//!
//! The warp runs inside `Runtime::step`, before it picks a unit. It
//! fires the timer wakes itself, and a timed wait's expiry writes the
//! observed bits back through the waiter's result pointer. Those bytes
//! belong to the step the warp then picked. A relation that never saw
//! them would call that step independent of another step that writes
//! the same address.
//!
//! Two waiters on one flag, with different deadlines and one result
//! pointer between them, is the smallest workload that shows it.

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
use cellgov_explore::execution::Execution;
use cellgov_explore::observer::observe_decisions;
use cellgov_lv2::PpuThreadAttrs;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::{Budget, InstructionCost};

const STEP_CAP: usize = 200;
const BUDGET: u64 = 4;
const FLAG_ID: u32 = 1;
/// Both waiters report through this one pointer, so their expiries
/// write the same bytes.
const SHARED_RESULT: u32 = 0x300;

const EARLY: UnitId = UnitId::new(0);
const LATE: UnitId = UnitId::new(1);

fn shared_range() -> ByteRange {
    ByteRange::new(GuestAddr::new(u64::from(SHARED_RESULT)), 8).unwrap()
}

/// Waits on a flag whose bits never arrive, with a finite timeout, so
/// the wait can only end in the warp and its expiry is what writes.
#[derive(Clone)]
struct FlagWaiter {
    id: UnitId,
    timeout_usec: u64,
    steps: Cell<u64>,
}

impl ExecutionUnit for FlagWaiter {
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
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        let (yield_reason, syscall_args) = if n == 1 {
            let mut args = [0u64; 9];
            args[0] = cellgov_ps3_abi::lv2::syscall::EVENT_FLAG_WAIT;
            args[1] = u64::from(FLAG_ID);
            args[2] = 0b10;
            args[3] = 0x01;
            args[4] = u64::from(SHARED_RESULT);
            args[5] = self.timeout_usec;
            (YieldReason::Syscall, Some(args))
        } else {
            (YieldReason::Finished, None)
        };
        ExecutionStepResult {
            yield_reason,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::with_pc(0x1000),
            fault: None,
            syscall_args,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

fn workload() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(4096), Budget::new(BUDGET), STEP_CAP);
    let early = rt.register_unit_with(|id| FlagWaiter {
        id,
        timeout_usec: 1_000,
        steps: Cell::new(0),
    });
    let late = rt.register_unit_with(|id| FlagWaiter {
        id,
        timeout_usec: 500_000,
        steps: Cell::new(0),
    });
    assert_eq!(early, EARLY, "registration order moved the early waiter");
    assert_eq!(late, LATE, "registration order moved the late waiter");

    // `expire_wait` resolves the waiter through its PPU thread record,
    // and writes nothing without one.
    let attrs = PpuThreadAttrs {
        entry: 0,
        arg: 0,
        stack_base: 0,
        stack_size: 0,
        priority: 0,
        tls_base: 0,
    };
    rt.lv2_host_mut()
        .seed_primary_ppu_thread(early, attrs.clone());
    rt.lv2_host_mut()
        .ppu_threads_mut()
        .create(late, attrs)
        .expect("a second thread id");
    rt.lv2_host_mut()
        .event_flags_mut()
        .create_with_id(FLAG_ID, 0)
        .expect("a fresh event-flag id");
    rt
}

#[test]
fn both_waits_park_and_only_the_warp_ends_them() {
    let mut rt = workload();
    for _ in 0..2 {
        let step = rt.step().expect("a waiter is runnable");
        rt.commit_step(&step.result, &step.effects)
            .expect("the wait commits");
    }
    assert_eq!(
        rt.registry().effective_status(EARLY),
        Some(UnitStatus::Blocked)
    );
    assert_eq!(
        rt.registry().effective_status(LATE),
        Some(UnitStatus::Blocked)
    );
    assert!(
        rt.memory()
            .read(shared_range())
            .unwrap()
            .iter()
            .all(|b| *b == 0),
        "nothing has written the shared result yet",
    );

    let step = rt.step().expect("the warp expires the earlier wait");
    rt.commit_step(&step.result, &step.effects)
        .expect("the woken step commits");
    assert!(
        rt.last_host_writes()
            .iter()
            .any(|(_, _, range)| *range == shared_range()),
        "the expiry wrote the shared result from inside the warp: {:?}",
        rt.last_host_writes(),
    );
}

#[test]
fn the_relation_holds_the_two_steps_the_warp_wrote_for() {
    let mut rt = workload();
    let (log, stop) = observe_decisions(&mut rt);
    assert!(!stop.is_truncated(), "a prefix answers for no pair: {stop}");
    let execution = Execution::from_log(&log);

    // `units_independent` answers `false` for a unit that ran no step,
    // so the verdict below stands on nothing until both units are in
    // the execution with their wait and their wake.
    assert_eq!(execution.events_of(EARLY).count(), 2);
    assert_eq!(execution.events_of(LATE).count(), 2);

    assert!(
        !execution.units_independent(EARLY, LATE),
        "each expiry wrote the same eight bytes, from inside the warp that \
         picked the step it belongs to",
    );
}
