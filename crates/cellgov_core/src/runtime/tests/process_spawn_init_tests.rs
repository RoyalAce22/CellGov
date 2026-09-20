//! A loader that stages an init pass parks the child's primary
//! behind a `PendingChildInit` until the host releases it.

use std::cell::Cell;

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_lv2::request::classify;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize};
use cellgov_time::{Budget, InstructionCost};

use super::super::spaces::AddressSpaceId;
use super::super::types::{PendingChildInit, ProcessSpawnLoadError, SpawnedProcessImage};
use super::super::Runtime;

#[derive(Clone)]
struct Idle {
    id: UnitId,
    finished: Cell<bool>,
}

impl ExecutionUnit for Idle {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.finished.get() {
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
        ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) {}
}

fn idle(id: UnitId) -> Idle {
    Idle {
        id,
        finished: Cell::new(false),
    }
}

const CHILD_PATH: &[u8] = b"/test/child.self";
const PID_OUT: u64 = 0x20;
const BLOCK: u64 = 0x40;
const PATH_STR: u64 = 0x80;
const EXPECTED_PID: u32 = cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID + 0x100;
const TOKEN: u64 = 0x00C0_FFEE;

fn build(init_token: Option<u64>) -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(4), 100);
    rt.registry_mut().register_with(idle);
    let write = |rt: &mut Runtime, addr: u64, bytes: &[u8]| {
        let range = ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).unwrap();
        rt.memory_mut().apply_commit(range, bytes).unwrap();
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
    rt.set_ppu_factory(|id, _init| Box::new(idle(id)));
    rt.set_process_spawn_loader(move |_bytes, mem, _space| {
        mem.install_region(0, 0x1000, "child", PageSize::Page64K)
            .map_err(|source| ProcessSpawnLoadError::RegionInstall { source })?;
        Ok(SpawnedProcessImage {
            entry_code: 0x100,
            entry_toc: 0x200,
            stack_top: 0xF00,
            lr_sentinel: 0,
            init_token,
        })
    });
    rt
}

fn spawn(rt: &mut Runtime) {
    let req = classify(
        cellgov_ps3_abi::lv2::syscall::PROCESS_SPAWN,
        &[PID_OUT, 1000, 0, BLOCK, 0x60, 0, 0, 0],
    );
    rt.dispatch_lv2_request(req, UnitId::new(0));
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(0),
        "spawn must succeed for the init handshake to be tested",
    );
}

#[test]
fn a_staged_init_parks_the_primary_and_queues_the_child() {
    let mut rt = build(Some(TOKEN));
    assert!(!rt.has_pending_child_init());
    spawn(&mut rt);

    let child = UnitId::new(1);
    assert_eq!(
        rt.registry().effective_status(child),
        Some(UnitStatus::Blocked),
        "primary must not run before the init pass",
    );
    assert!(rt.has_pending_child_init());
    let pending = rt.take_pending_child_inits();
    assert_eq!(
        pending,
        vec![PendingChildInit {
            pid: EXPECTED_PID,
            space: AddressSpaceId::new(1),
            primary_unit: child,
            init_token: TOKEN,
        }]
    );
    assert!(!rt.has_pending_child_init(), "take drains the queue");
    assert!(rt.take_pending_child_inits().is_empty());
}

#[test]
fn releasing_the_child_makes_its_primary_runnable() {
    let mut rt = build(Some(TOKEN));
    spawn(&mut rt);
    let [pending] = rt.take_pending_child_inits()[..] else {
        panic!("exactly one child parked");
    };
    rt.release_child_init(pending.primary_unit);
    assert_eq!(
        rt.registry().effective_status(pending.primary_unit),
        Some(UnitStatus::Runnable)
    );
}

#[test]
fn children_are_queued_in_spawn_order() {
    let mut rt = build(Some(TOKEN));
    spawn(&mut rt);
    spawn(&mut rt);
    let pending = rt.take_pending_child_inits();
    assert_eq!(
        pending,
        vec![
            PendingChildInit {
                pid: EXPECTED_PID,
                space: AddressSpaceId::new(1),
                primary_unit: UnitId::new(1),
                init_token: TOKEN,
            },
            PendingChildInit {
                pid: EXPECTED_PID + 0x100,
                space: AddressSpaceId::new(2),
                primary_unit: UnitId::new(2),
                init_token: TOKEN,
            },
        ]
    );
    for child in pending {
        assert_eq!(
            rt.registry().effective_status(child.primary_unit),
            Some(UnitStatus::Blocked),
            "every parked primary stays Blocked until its own release",
        );
    }
}

#[test]
fn a_child_that_exited_during_its_init_pass_is_not_resumed_by_release() {
    let mut rt = build(Some(TOKEN));
    spawn(&mut rt);
    let [pending] = rt.take_pending_child_inits()[..] else {
        panic!("exactly one child parked");
    };

    // A unit bound to the child's pid exits the process while the
    // primary is still parked; the exit sweep finishes the primary.
    let exit = classify(
        cellgov_ps3_abi::lv2::syscall::PROCESS_EXIT,
        &[7, 0, 0, 0, 0, 0, 0, 0],
    );
    rt.dispatch_lv2_request(exit, pending.primary_unit);
    assert_eq!(rt.lv2_host().process_exit_status(pending.pid), Some(7));
    assert_eq!(
        rt.registry().effective_status(pending.primary_unit),
        Some(UnitStatus::Finished)
    );

    rt.release_child_init(pending.primary_unit);
    assert_eq!(
        rt.registry().effective_status(pending.primary_unit),
        Some(UnitStatus::Finished),
        "release must not resume a thread of an exited process",
    );
    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("runtime.child_init_release_of_unparked_primary"),
        1
    );
}

#[test]
fn releasing_a_primary_that_was_never_parked_is_witnessed() {
    let mut rt = build(None);
    spawn(&mut rt);
    let child = UnitId::new(1);
    rt.release_child_init(child);
    assert_eq!(
        rt.registry().effective_status(child),
        Some(UnitStatus::Runnable)
    );
    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("runtime.child_init_release_of_unparked_primary"),
        1
    );
}

#[test]
fn a_second_release_of_the_same_child_is_witnessed_not_silent() {
    let mut rt = build(Some(TOKEN));
    spawn(&mut rt);
    let [pending] = rt.take_pending_child_inits()[..] else {
        panic!("exactly one child parked");
    };
    rt.release_child_init(pending.primary_unit);
    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("runtime.child_init_release_of_unparked_primary"),
        0
    );
    rt.release_child_init(pending.primary_unit);
    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("runtime.child_init_release_of_unparked_primary"),
        1
    );
}

#[test]
fn a_loader_without_a_staged_init_starts_the_primary_at_once() {
    let mut rt = build(None);
    spawn(&mut rt);
    assert_eq!(
        rt.registry().effective_status(UnitId::new(1)),
        Some(UnitStatus::Runnable)
    );
    assert!(!rt.has_pending_child_init());
}

#[test]
fn a_parked_child_survives_snapshot_and_restore() {
    let mut rt = build(Some(TOKEN));
    spawn(&mut rt);
    let snap = rt.snapshot();
    let taken = rt.take_pending_child_inits();
    assert_eq!(taken.len(), 1);
    rt.restore_into(&snap);
    assert!(
        rt.has_pending_child_init(),
        "queue restored with the override"
    );
    assert_eq!(rt.take_pending_child_inits(), taken);
    assert_eq!(
        rt.registry().effective_status(UnitId::new(1)),
        Some(UnitStatus::Blocked)
    );
}
