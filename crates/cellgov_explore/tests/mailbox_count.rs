//! Whether a step that reads its own mailbox's occupancy reaches the
//! relation.
//!
//! The runtime hands every step of a unit the occupancy of the mailbox
//! that shares its id, and an SPU's `rchcnt SPU_RdInMbox` returns it.
//! The step emits no effect naming the mailbox, so a send to it and the
//! read meet only in the runtime's record of the read.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use std::cell::Cell;

use cellgov_core::Runtime;
use cellgov_effects::{Effect, MailboxMessage, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_explore::config::ExplorationConfig;
use cellgov_explore::execution::Execution;
use cellgov_explore::explorer::explore_window;
use cellgov_explore::observer::observe_decisions;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_sync::MailboxId;
use cellgov_testkit::world::WritingUnit;
use cellgov_time::{Budget, GuestTicks, InstructionCost};

const STEP_CAP: usize = 200;
const BUDGET: u64 = 16;

const COUNTER: UnitId = UnitId::new(0);
const SENDER: UnitId = UnitId::new(1);
const LONER: UnitId = UnitId::new(2);

fn counter_mailbox() -> MailboxId {
    MailboxId::new(COUNTER.raw())
}

fn count_range() -> ByteRange {
    ByteRange::new(GuestAddr::new(0), 4).unwrap()
}

fn elsewhere_range() -> ByteRange {
    ByteRange::new(GuestAddr::new(64), 8).unwrap()
}

/// Stores the occupancy of its own mailbox once, then finishes: the
/// read an SPU's `rchcnt SPU_RdInMbox` makes.
#[derive(Clone)]
struct Counter {
    id: UnitId,
    steps: Cell<u8>,
}

impl ExecutionUnit for Counter {
    type Snapshot = u8;

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
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.steps.set(self.steps.get() + 1);
        effects.push(Effect::shared_write(
            count_range(),
            WritePayload::from_slice(&ctx.mailbox_occupancy().to_be_bytes()),
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

    fn snapshot(&self) -> u8 {
        self.steps.get()
    }
}

/// Sends one message to the counter's mailbox, then finishes.
#[derive(Clone)]
struct Sender {
    id: UnitId,
    steps: Cell<u8>,
}

impl ExecutionUnit for Sender {
    type Snapshot = u8;

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
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.steps.set(self.steps.get() + 1);
        effects.push(Effect::MailboxSend {
            mailbox: counter_mailbox(),
            message: MailboxMessage::new(7),
            source: self.id,
        });
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u8 {
        self.steps.get()
    }
}

/// The counter, the unit that fills its mailbox, and one unit that
/// shares nothing with either.
fn workload() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(BUDGET), STEP_CAP);
    let counter = rt.register_unit_with(|id| Counter {
        id,
        steps: Cell::new(0),
    });
    assert_eq!(counter, COUNTER, "registration order moved the counter");
    assert_eq!(rt.mailbox_registry_mut().register(4), counter_mailbox());
    let sender = rt.register_unit_with(|id| Sender {
        id,
        steps: Cell::new(0),
    });
    let loner = rt.register_unit_with(|id| WritingUnit::of_value(id, 1, elsewhere_range(), 0x11));
    assert_eq!(sender, SENDER, "registration order moved the sender");
    assert_eq!(loner, LONER, "registration order moved the loner");
    rt
}

#[test]
fn the_relation_holds_a_send_against_the_occupancy_read_it_changes() {
    let mut rt = workload();
    let (log, stop) = observe_decisions(&mut rt);
    assert!(!stop.is_truncated(), "a prefix answers for no pair: {stop}");
    let execution = Execution::from_log(&log);

    assert!(
        !execution.units_independent(SENDER, COUNTER),
        "the send changes the occupancy the counter stores",
    );
    // Without this the case above passes under a relation that pairs
    // every step with every other.
    assert!(
        execution.units_independent(COUNTER, LONER),
        "the loner writes bytes the counter never reads, and sends nothing",
    );
}

#[test]
fn the_verdict_reads_schedule_sensitive() {
    let result = explore_window(workload, &ExplorationConfig::default());
    assert_eq!(
        result.outcome,
        cellgov_explore::classify::OutcomeClass::ScheduleSensitive,
        "the count is 0 before the send and 1 after it",
    );
}

#[test]
fn a_count_read_conflicts_with_a_send_or_a_receive_on_its_mailbox_either_way() {
    use cellgov_explore::StepFootprint;
    let count = StepFootprint {
        mailbox_counts: vec![counter_mailbox()],
        ..Default::default()
    };
    let send = StepFootprint {
        mailbox_sends: vec![counter_mailbox()],
        ..Default::default()
    };
    let receive = StepFootprint {
        mailbox_receives: vec![counter_mailbox()],
        ..Default::default()
    };
    for peer in [&send, &receive] {
        assert!(count.conflicts(peer), "{peer:?}");
        assert!(peer.conflicts(&count), "{peer:?}");
    }
    let elsewhere = StepFootprint {
        mailbox_sends: vec![MailboxId::new(9)],
        mailbox_receives: vec![MailboxId::new(9)],
        ..Default::default()
    };
    assert!(!count.conflicts(&elsewhere) && !elsewhere.conflicts(&count));
}
