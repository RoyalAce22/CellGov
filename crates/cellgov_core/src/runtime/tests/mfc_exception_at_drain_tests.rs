//! A run that stops with a refused command still queued has that
//! command's exception at a drain, and reading it changes nothing.

use cellgov_dma::{InvalidMfcCommand, MfcCommandError, MfcParameters};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::GuestMemory;
use cellgov_time::{Budget, InstructionCost};

use crate::runtime::state::Runtime;
use crate::runtime::StepError;

/// Queues one command the MFC refuses, then finishes.
#[derive(Clone)]
struct RefusedCommandUnit {
    id: UnitId,
    done: bool,
}

const REFUSED: InvalidMfcCommand = InvalidMfcCommand {
    word: 0x20,
    params: MfcParameters {
        lsa: 0x100,
        eah: 0,
        eal: 0x2000,
        size: 3,
        tag: 1,
    },
    error: MfcCommandError::SizeUnaligned(3),
};

impl ExecutionUnit for RefusedCommandUnit {
    type Snapshot = bool;
    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        if self.done {
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
        self.done = true;
        effects.push(Effect::MfcInvalidCommand {
            issuer: self.id,
            command: REFUSED,
        });
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(1),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }
    fn snapshot(&self) -> bool {
        self.done
    }
}

#[test]
fn a_refused_command_the_queue_never_reached_is_the_exception_at_a_drain() {
    let mut rt = Runtime::new(GuestMemory::new(0x100), Budget::new(4), 100);
    let unit = rt
        .registry_mut()
        .register_with(|id| RefusedCommandUnit { id, done: false });
    let terminal = loop {
        match rt.step() {
            Ok(step) => {
                rt.commit_step(&step.result, &step.effects)
                    .expect("the batch commits");
            }
            Err(err) => break err,
        }
    };
    assert_eq!(terminal, StepError::NoRunnableUnit);
    let queued = rt.dma_queue().len();
    let hash = rt.sync_state_hash();

    let exception = rt
        .mfc_exception_at_drain()
        .expect("the queued command would raise");
    assert_eq!(exception.unit, unit);
    assert_eq!(exception.command, REFUSED);

    assert_eq!(rt.dma_queue().len(), queued, "nothing drained");
    assert_eq!(rt.sync_state_hash(), hash, "the state is unchanged");
    assert_eq!(
        rt.take_mfc_exception(),
        None,
        "and nothing was recorded: the queue never reached the command"
    );
}
