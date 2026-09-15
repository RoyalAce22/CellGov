//! A window that spans a spawn refuses instead of pruning.
//!
//! The host's child-init pass holds every other runnable unit `Blocked`
//! across the child's `module_start` and restores them after, through
//! no effect. No footprint records either half, so a relation asked
//! about the steps around it would call independent two steps that the
//! pass separated. That is a false independence, which is the silent
//! direction.
//!
//! No explorer runs the pass, so the search cannot model the window
//! either way. It stops and says so.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use std::cell::Cell;

use cellgov_core::{AddressSpaceId, ProcessSpawnLoadError, Runtime, SpawnedProcessImage};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_explore::config::ExplorationConfig;
use cellgov_explore::explorer::explore_window;
use cellgov_explore::observer::observe_decisions;
use cellgov_explore::util::{run_to_stall, StopClass, StopReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize};
use cellgov_time::{Budget, InstructionCost};

const CHILD_PATH: &[u8] = b"/test/child.self";
const PID_OUT: u64 = 0x20;
const BLOCK: u64 = 0x40;
const PATH_STR: u64 = 0x80;
const TOKEN: u64 = 0x00C0_FFEE;

/// Spawns a child on its second step, then finishes.
///
/// The first step is an ordinary one, so the window has a step before
/// the spawn and the refusal is not the very first thing it meets.
#[derive(Clone)]
struct SpawningUnit {
    id: UnitId,
    steps: Cell<u64>,
}

impl ExecutionUnit for SpawningUnit {
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
        if n == 1 {
            return ExecutionStepResult {
                yield_reason: YieldReason::BudgetExhausted,
                consumed_cost: InstructionCost::new(budget.raw()),
                local_diagnostics: LocalDiagnostics::empty(),
                fault: None,
                syscall_args: None,
            };
        }
        let mut args = [0u64; 9];
        args[0] = cellgov_ps3_abi::lv2::syscall::PROCESS_SPAWN;
        args[1] = PID_OUT;
        args[2] = 1000;
        args[4] = BLOCK;
        args[5] = 0x60;
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

/// A second unit, so the window holds a branching point of its own.
#[derive(Clone)]
struct Idle {
    id: UnitId,
    steps: Cell<u64>,
}

impl ExecutionUnit for Idle {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= 3 {
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
        ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
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

fn spawns_a_child() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(4), 200);
    rt.register_unit_with(|id| SpawningUnit {
        id,
        steps: Cell::new(0),
    });
    rt.register_unit_with(|id| Idle {
        id,
        steps: Cell::new(0),
    });

    // `place_bytes` is the one host write a driving program makes on
    // its own account, which is what this fixture is.
    let write = |rt: &mut Runtime, addr: u64, bytes: &[u8]| {
        let range = ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).unwrap();
        rt.place_bytes(AddressSpaceId::BOOT, range, bytes).unwrap();
    };
    write(&mut rt, BLOCK, &16u64.to_be_bytes());
    write(&mut rt, BLOCK + 16, &PATH_STR.to_be_bytes());
    write(&mut rt, BLOCK + 24, &0u64.to_be_bytes());
    let mut path = CHILD_PATH.to_vec();
    path.push(0);
    write(&mut rt, PATH_STR, &path);

    rt.lv2_host_mut()
        .content_store_mut()
        .register(CHILD_PATH, vec![0xEE; 16]);
    rt.set_ppu_factory(|id, _init| {
        Box::new(Idle {
            id,
            steps: Cell::new(0),
        })
    });
    rt.set_process_spawn_loader(move |_bytes, mem| {
        mem.install_region(0, 0x1000, "child", PageSize::Page64K)
            .map_err(|source| ProcessSpawnLoadError::RegionInstall { source })?;
        Ok(SpawnedProcessImage {
            entry_code: 0x100,
            entry_toc: 0x200,
            stack_top: 0xF00,
            lr_sentinel: 0,
            init_token: Some(TOKEN),
        })
    });
    rt
}

/// The premise: the workload really does park a child behind a staged
/// init pass, part-way through rather than at the start.
#[test]
fn the_workload_stages_a_child_init_mid_run() {
    let mut rt = spawns_a_child();
    assert!(!rt.has_pending_child_init());

    let mut steps = 0;
    while !rt.has_pending_child_init() {
        let step = rt.step().expect("the workload runs to its spawn");
        rt.commit_step(&step.result, &step.effects).unwrap();
        steps += 1;
        assert!(steps < 10, "the spawn should arrive well inside this");
    }
    assert!(
        steps > 1,
        "the spawn is not the first step, so the window covers a step before it",
    );
}

#[test]
fn run_to_stall_refuses_a_window_that_spans_a_spawn() {
    let mut rt = spawns_a_child();
    let stop = run_to_stall(&mut rt, 200);
    assert_eq!(stop, StopReason::ChildInitUnserved);
    assert!(
        stop.is_truncated(),
        "the run answers for a prefix, not the workload",
    );
    assert_eq!(
        stop.class(),
        StopClass::Unserved,
        "nothing is wrong with the model or the guest: the window holds what \
         the relation cannot see",
    );
    assert_ne!(
        stop.class(),
        StopClass::Refusal,
        "a reader chasing a model defect must not be sent here",
    );
}

#[test]
fn the_observer_refuses_it_the_same_way() {
    let mut rt = spawns_a_child();
    let (log, stop) = observe_decisions(&mut rt);
    assert_eq!(stop, StopReason::ChildInitUnserved);
    assert!(
        !log.is_empty(),
        "the steps before the spawn are recorded; the refusal is what follows",
    );
}

/// The same workload, advanced to its spawn by hand, which is the shape
/// a driver hands over when it opens a window past one.
fn staged_child_init() -> Runtime {
    let mut rt = spawns_a_child();
    while !rt.has_pending_child_init() {
        let step = rt.step().expect("the workload runs to its spawn");
        rt.commit_step(&step.result, &step.effects).unwrap();
    }
    rt
}

/// A pass already pending costs no step: the parks are in force before
/// the first step of the window, so no relation covers it either.
#[test]
fn a_pass_pending_at_entry_refuses_before_a_step_runs() {
    let mut rt = staged_child_init();
    let opened_at = rt.steps_taken();
    assert_eq!(run_to_stall(&mut rt, 200), StopReason::ChildInitUnserved);
    assert_eq!(rt.steps_taken(), opened_at, "no step ran under the parks");

    let mut rt = staged_child_init();
    let opened_at = rt.steps_taken();
    let (log, stop) = observe_decisions(&mut rt);
    assert_eq!(stop, StopReason::ChildInitUnserved);
    assert!(log.is_empty(), "the window holds no decision point");
    assert_eq!(rt.steps_taken(), opened_at);

    let result = explore_window(staged_child_init, &ExplorationConfig::default());
    assert_eq!(result.baseline_stop, StopReason::ChildInitUnserved);
    assert_eq!(
        result.baseline_steps, 0,
        "a baseline that ran nothing measures nothing"
    );
    assert_eq!(
        result.outcome,
        cellgov_explore::classify::OutcomeClass::Inconclusive
    );
}

/// The search reaches no verdict over a window it cannot reason about.
#[test]
fn the_search_claims_nothing_over_a_window_that_spans_a_spawn() {
    let result = explore_window(spawns_a_child, &ExplorationConfig::default());
    assert_eq!(result.baseline_stop, StopReason::ChildInitUnserved);
    assert!(result.bounds_hit, "a prefix baseline bounds the search");
    assert_eq!(
        result.classes_explored, None,
        "a prefix covers no class, so the search counts none",
    );
    assert_eq!(
        result.outcome,
        cellgov_explore::classify::OutcomeClass::Inconclusive,
        "no verdict rests on a relation that cannot see the pass",
    );
}
