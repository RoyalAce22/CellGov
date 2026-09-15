//! Whether a wake an LV2 handler performs reaches the relation.
//!
//! The write half of this lives in `lv2_out_param.rs`. A handler can
//! also send a mailbox message, which lands no guest byte and releases
//! a parked unit. `sys_spu_thread_write_in_mbox` is the one arm that
//! does. A record of the writes alone cannot see it.
//!
//! Both halves ride the same record of what the dispatch applied. The
//! send answers to the mailbox clause every unit-emitted send already
//! answers to, and to the wake clause as well. The handler's send reads
//! the target's status alone and releases its park, so a park no
//! mailbox pairs ends with that send.

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
use cellgov_explore::prescribed::PrescribedScheduler;
use cellgov_lv2::thread_group::MAX_SLOTS_PER_GROUP;
use cellgov_lv2::GroupState;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_sync::MailboxId;
use cellgov_testkit::world::WritingUnit;
use cellgov_time::{Budget, InstructionCost};

const STEP_CAP: usize = 200;
const BUDGET: u64 = 16;
const MESSAGE: u64 = 0x5150;

const RECEIVER: UnitId = UnitId::new(0);
const SENDER: UnitId = UnitId::new(1);
const LONER: UnitId = UnitId::new(2);

/// The handler sends to the mailbox keyed by the target unit's id.
fn receiver_mailbox() -> MailboxId {
    MailboxId::new(RECEIVER.raw())
}

/// A second mailbox, which no handler fills. The receiver parks on this
/// one in the workload that shows how the release ignores the park's
/// reason. Its raw value is the registry's next sequential id.
fn spare_mailbox() -> MailboxId {
    MailboxId::new(1)
}

fn elsewhere_range() -> ByteRange {
    ByteRange::new(GuestAddr::new(64), 8).unwrap()
}

/// Waits on `wait_on`, then finishes once it runs again.
#[derive(Clone)]
struct Waiter {
    id: UnitId,
    wait_on: MailboxId,
    phase: Cell<u8>,
}

impl ExecutionUnit for Waiter {
    type Snapshot = u8;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.phase.get() >= 2 {
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
        let p = self.phase.get();
        self.phase.set(p + 1);
        if p == 0 {
            effects.push(Effect::MailboxReceiveAttempt {
                mailbox: self.wait_on,
                source: self.id,
            });
            return ExecutionStepResult {
                yield_reason: YieldReason::MailboxAccess,
                consumed_cost: InstructionCost::new(budget.raw()),
                local_diagnostics: LocalDiagnostics::empty(),
                fault: None,
                syscall_args: None,
            };
        }
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u8 {
        self.phase.get()
    }
}

/// Calls `sys_spu_thread_write_in_mbox` once, then finishes.
///
/// It emits no effect of its own: the send is the handler's.
#[derive(Clone)]
struct MbSender {
    id: UnitId,
    thread_id: u32,
    steps: Cell<u64>,
}

impl ExecutionUnit for MbSender {
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
        args[0] = cellgov_ps3_abi::lv2::syscall::SPU_THREAD_WRITE_MB;
        args[1] = u64::from(self.thread_id);
        args[2] = MESSAGE;
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

/// A waiter parked on its mailbox, the unit whose syscall fills it, and
/// one unit that shares nothing with either.
fn workload() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(BUDGET), STEP_CAP);
    let receiver = rt.register_unit_with(|id| Waiter {
        id,
        wait_on: receiver_mailbox(),
        phase: Cell::new(0),
    });
    assert_eq!(receiver, RECEIVER, "registration order moved the receiver");

    let mailbox = rt.mailbox_registry_mut().register(4);
    assert_eq!(
        mailbox,
        receiver_mailbox(),
        "the handler sends to the mailbox keyed by the receiver's id",
    );

    // `dispatch_write_mb` sends only for a thread whose group is
    // running. The arm refuses with ESRCH at `Created`, so this
    // workload puts the group in `Running`.
    let groups = rt.lv2_host_mut().thread_groups_mut();
    let group = groups.create(1).unwrap();
    groups.get_mut(group).unwrap().state = GroupState::Running;
    groups.record_spu(RECEIVER, group, 0).unwrap();
    let thread_id = group * MAX_SLOTS_PER_GROUP;

    let sender = rt.register_unit_with(|id| MbSender {
        id,
        thread_id,
        steps: Cell::new(0),
    });
    let loner = rt.register_unit_with(|id| WritingUnit::of_value(id, 1, elsewhere_range(), 0x11));
    assert_eq!(sender, SENDER, "registration order moved the sender");
    assert_eq!(loner, LONER, "registration order moved the loner");
    rt
}

/// The same three units, with the receiver parked on a mailbox no
/// handler fills.
///
/// `Runtime::apply_lv2_effects` reads the target's status alone, so this
/// park ends too, and no mailbox pairs the two steps.
fn workload_parked_elsewhere() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(BUDGET), STEP_CAP);
    let receiver = rt.register_unit_with(|id| Waiter {
        id,
        wait_on: spare_mailbox(),
        phase: Cell::new(0),
    });
    assert_eq!(receiver, RECEIVER, "registration order moved the receiver");

    let filled = rt.mailbox_registry_mut().register(4);
    assert_eq!(
        filled,
        receiver_mailbox(),
        "the handler sends to the mailbox keyed by the receiver's id",
    );
    let parked_on = rt.mailbox_registry_mut().register(4);
    assert_eq!(
        parked_on,
        spare_mailbox(),
        "the receiver parks on the second registration",
    );

    let groups = rt.lv2_host_mut().thread_groups_mut();
    let group = groups.create(1).unwrap();
    groups.get_mut(group).unwrap().state = GroupState::Running;
    groups.record_spu(RECEIVER, group, 0).unwrap();
    let thread_id = group * MAX_SLOTS_PER_GROUP;

    let sender = rt.register_unit_with(|id| MbSender {
        id,
        thread_id,
        steps: Cell::new(0),
    });
    let loner = rt.register_unit_with(|id| WritingUnit::of_value(id, 1, elsewhere_range(), 0x11));
    assert_eq!(sender, SENDER, "registration order moved the sender");
    assert_eq!(loner, LONER, "registration order moved the loner");
    rt
}

#[test]
fn the_handler_sends_a_message_the_senders_own_step_never_names() {
    let mut rt = workload();
    rt.set_scheduler(PrescribedScheduler::new(vec![Some(RECEIVER), Some(SENDER)]));

    let park = rt.step().expect("the receiver is runnable");
    assert_eq!(park.unit, RECEIVER);
    rt.commit_step(&park.result, &park.effects)
        .expect("the receive attempt commits");
    assert_eq!(
        rt.registry().effective_status(RECEIVER),
        Some(UnitStatus::Blocked),
        "an empty mailbox parks the receiver, which is what the send releases",
    );

    let send = rt.step().expect("the sender is runnable");
    assert_eq!(send.unit, SENDER);
    assert!(
        send.effects.is_empty(),
        "the syscall step emits nothing itself: {:?}",
        send.effects,
    );
    rt.commit_step(&send.result, &send.effects)
        .expect("the dispatch commits");

    assert!(
        rt.last_lv2_effects().iter().any(|effect| matches!(
            effect,
            Effect::MailboxSend { mailbox, .. } if *mailbox == receiver_mailbox()
        )),
        "the runtime publishes the send the handler made: {:?}",
        rt.last_lv2_effects(),
    );
    assert_ne!(
        rt.registry().effective_status(RECEIVER),
        Some(UnitStatus::Blocked),
        "and the send released the park",
    );
}

#[test]
fn the_relation_holds_the_sender_against_the_unit_its_handler_wakes() {
    let mut rt = workload();
    let (log, stop) = observe_decisions(&mut rt);
    assert!(!stop.is_truncated(), "a prefix answers for no pair: {stop}");
    let execution = Execution::from_log(&log);

    assert!(
        !execution.units_independent(SENDER, RECEIVER),
        "the handler's send is what lets the receiver run again",
    );
    // Without this the case above passes under a relation that pairs
    // every step with every other.
    assert!(
        execution.units_independent(RECEIVER, LONER),
        "the loner writes bytes the receiver never reads, and sends nothing",
    );
}

#[test]
fn the_release_ends_a_park_on_a_mailbox_the_handler_never_fills() {
    let mut rt = workload_parked_elsewhere();
    rt.set_scheduler(PrescribedScheduler::new(vec![Some(RECEIVER), Some(SENDER)]));

    let park = rt.step().expect("the receiver is runnable");
    assert_eq!(park.unit, RECEIVER);
    rt.commit_step(&park.result, &park.effects)
        .expect("the receive attempt commits");
    assert_eq!(
        rt.registry().effective_status(RECEIVER),
        Some(UnitStatus::Blocked),
        "the spare mailbox is empty, so the receiver parks on it",
    );

    let send = rt.step().expect("the sender is runnable");
    assert_eq!(send.unit, SENDER);
    rt.commit_step(&send.result, &send.effects)
        .expect("the dispatch commits");

    let sends: Vec<MailboxId> = rt
        .last_lv2_effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::MailboxSend { mailbox, .. } => Some(*mailbox),
            _ => None,
        })
        .collect();
    assert_eq!(
        sends,
        vec![receiver_mailbox()],
        "the handler fills the mailbox keyed by the receiver's id, never the \
         one the receiver parked on",
    );
    assert_ne!(
        rt.registry().effective_status(RECEIVER),
        Some(UnitStatus::Blocked),
        "and the park ended anyway",
    );
}

/// The mailbox clause alone would call these two steps independent, and
/// prune the order where the receiver never runs again.
#[test]
fn the_relation_holds_the_sender_against_a_park_no_mailbox_pairs() {
    let mut rt = workload_parked_elsewhere();
    rt.set_scheduler(PrescribedScheduler::new(vec![
        Some(RECEIVER),
        Some(SENDER),
        Some(RECEIVER),
        Some(LONER),
    ]));
    let (log, stop) = observe_decisions(&mut rt);
    assert!(!stop.is_truncated(), "a prefix answers for no pair: {stop}");
    let execution = Execution::from_log(&log);

    assert!(
        !execution.units_independent(SENDER, RECEIVER),
        "the handler's send is what lets the receiver run again, whatever \
         mailbox the receiver parked on",
    );
    // Without this the case above passes under a relation that pairs
    // every step with every other.
    assert!(
        execution.units_independent(RECEIVER, LONER),
        "the loner writes bytes the receiver never reads, and sends nothing",
    );
}
