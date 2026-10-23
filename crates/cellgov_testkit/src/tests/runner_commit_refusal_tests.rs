//! A refused commit ends the scenario run.

use super::*;
use cellgov_core::Runtime;
use cellgov_effects::{Effect, MailboxMessage};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_sync::MailboxId;
use cellgov_time::{Budget, InstructionCost};

const UNREGISTERED: MailboxId = MailboxId::new(9);

/// Sends to a mailbox nobody registered on each of three steps.
#[derive(Clone)]
struct UnregisteredSender {
    id: UnitId,
    steps: u32,
}

impl ExecutionUnit for UnregisteredSender {
    type Snapshot = u32;
    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        if self.steps >= 3 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }
    fn run_until_yield(
        &mut self,
        _budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.steps += 1;
        effects.push(Effect::MailboxSend {
            mailbox: UNREGISTERED,
            message: MailboxMessage::new(1),
            source: self.id,
        });
        ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_cost: InstructionCost::new(1),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }
    fn snapshot(&self) -> u32 {
        self.steps
    }
}

#[test]
fn a_refused_commit_stops_the_run_and_names_the_refusal() {
    let result = run(ScenarioFixture::builder()
        .memory_size(16)
        .max_steps(100)
        .register(|rt: &mut Runtime| {
            rt.register_unit_with(|id| UnregisteredSender { id, steps: 0 });
        })
        .build());

    assert_eq!(
        result.outcome,
        ScenarioOutcome::CommitRefused(CommitError::UnknownMailbox {
            effect_index: 0,
            mailbox: UNREGISTERED,
        })
    );
    assert_eq!(result.steps_taken, 1);
}
