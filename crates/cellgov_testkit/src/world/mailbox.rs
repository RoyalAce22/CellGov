//! The mailbox fixtures: a producer, and a sender paired with a responder.

use cellgov_effects::{Effect, MailboxMessage};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_sync::MailboxId;
use cellgov_time::{Budget, InstructionCost};
use std::cell::Cell;

/// Emits one [`Effect::MailboxSend`] per step into `target` with message
/// words `1..=max`; finishes after `max` steps.
#[derive(Clone)]
pub struct MailboxProducer {
    id: UnitId,
    target: MailboxId,
    steps: Cell<u64>,
    max: u64,
}

impl MailboxProducer {
    /// Construct a producer sending `max` messages into `target`.
    pub fn new(id: UnitId, target: MailboxId, max: u64) -> Self {
        Self {
            id,
            target,
            steps: Cell::new(0),
            max,
        }
    }
}

impl ExecutionUnit for MailboxProducer {
    type Snapshot = u64;
    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        if self.steps.get() >= self.max {
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
        let yield_reason = if n >= self.max {
            YieldReason::Finished
        } else {
            YieldReason::MailboxAccess
        };
        effects.push(Effect::MailboxSend {
            mailbox: self.target,
            message: MailboxMessage::new(n as u32),
            source: self.id,
        });
        ExecutionStepResult {
            yield_reason,
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

/// Three-stage PPU-like sender: send command + wake responder + wait;
/// receive attempt; consume response and emit a `TraceMarker`.
///
/// The sender emits an explicit `WakeUnit`: the commit pipeline does not
/// wake a unit on message delivery.
#[derive(Clone)]
pub struct MailboxSender {
    id: UnitId,
    responder: UnitId,
    cmd_mailbox: MailboxId,
    resp_mailbox: MailboxId,
    command: u32,
    phase: Cell<u8>,
}

impl MailboxSender {
    /// Construct a sender.
    pub fn new(
        id: UnitId,
        responder: UnitId,
        cmd_mailbox: MailboxId,
        resp_mailbox: MailboxId,
        command: u32,
    ) -> Self {
        Self {
            id,
            responder,
            cmd_mailbox,
            resp_mailbox,
            command,
            phase: Cell::new(0),
        }
    }
}

impl ExecutionUnit for MailboxSender {
    type Snapshot = u8;
    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        if self.phase.get() >= 3 {
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
        let p = self.phase.get();
        self.phase.set(p + 1);
        match p {
            0 => {
                effects.push(Effect::MailboxSend {
                    mailbox: self.cmd_mailbox,
                    message: MailboxMessage::new(self.command),
                    source: self.id,
                });
                effects.push(Effect::WakeUnit {
                    target: self.responder,
                    source: self.id,
                });
                effects.push(Effect::WaitOnEvent {
                    target: cellgov_effects::WaitTarget::Mailbox(self.resp_mailbox),
                    source: self.id,
                });
                ExecutionStepResult {
                    yield_reason: YieldReason::MailboxAccess,
                    consumed_cost: InstructionCost::new(budget.raw()),
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                    syscall_args: None,
                }
            }
            1 => {
                effects.push(Effect::MailboxReceiveAttempt {
                    mailbox: self.resp_mailbox,
                    source: self.id,
                });
                ExecutionStepResult {
                    yield_reason: YieldReason::MailboxAccess,
                    consumed_cost: InstructionCost::new(budget.raw()),
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                    syscall_args: None,
                }
            }
            _ => {
                let response = ctx.received_messages().first().copied().unwrap_or(0);
                effects.push(Effect::TraceMarker {
                    marker: response,
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
        }
    }
    fn snapshot(&self) -> u8 {
        self.phase.get()
    }
}

/// Two-stage SPU-like responder paired with [`MailboxSender`]: receive
/// attempt; then read command, send `command + 1` response, wake sender.
#[derive(Clone)]
pub struct MailboxResponder {
    id: UnitId,
    sender: UnitId,
    cmd_mailbox: MailboxId,
    resp_mailbox: MailboxId,
    phase: Cell<u8>,
}

impl MailboxResponder {
    /// Construct a responder.
    pub fn new(
        id: UnitId,
        sender: UnitId,
        cmd_mailbox: MailboxId,
        resp_mailbox: MailboxId,
    ) -> Self {
        Self {
            id,
            sender,
            cmd_mailbox,
            resp_mailbox,
            phase: Cell::new(0),
        }
    }
}

impl ExecutionUnit for MailboxResponder {
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
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let p = self.phase.get();
        self.phase.set(p + 1);
        match p {
            0 => {
                effects.push(Effect::MailboxReceiveAttempt {
                    mailbox: self.cmd_mailbox,
                    source: self.id,
                });
                ExecutionStepResult {
                    yield_reason: YieldReason::MailboxAccess,
                    consumed_cost: InstructionCost::new(budget.raw()),
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                    syscall_args: None,
                }
            }
            _ => {
                let cmd = ctx.received_messages().first().copied().unwrap_or(0);
                let response = cmd.wrapping_add(1);
                effects.push(Effect::MailboxSend {
                    mailbox: self.resp_mailbox,
                    message: MailboxMessage::new(response),
                    source: self.id,
                });
                effects.push(Effect::WakeUnit {
                    target: self.sender,
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
        }
    }
    fn snapshot(&self) -> u8 {
        self.phase.get()
    }
}
