//! An exploration names the MFC exception a run raised: the run stalls
//! like any other, so the result's line is the only place the cause
//! survives.

use crate::backtrack::explore_backtrack;
use crate::config::ExplorationConfig;
use crate::explorer::explore;
use cellgov_core::Runtime;
use cellgov_dma::{InvalidMfcCommand, MfcCommandError, MfcParameters};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::GuestMemory;
use cellgov_time::{Budget, InstructionCost};

/// Spends `wait` one-tick steps, queues one command the MFC refuses,
/// then finishes.
#[derive(Clone)]
struct RefusedCommandUnit {
    id: UnitId,
    wait: u32,
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
        let yield_reason = if self.wait > 0 {
            self.wait -= 1;
            YieldReason::BudgetExhausted
        } else {
            self.done = true;
            effects.push(Effect::MfcInvalidCommand {
                issuer: self.id,
                command: REFUSED,
            });
            YieldReason::Finished
        };
        ExecutionStepResult {
            yield_reason,
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

/// Two writers over disjoint bytes, so the workload branches, and a unit
/// that queues a refused command once both finish.
///
/// Every step costs one tick, so the run stops with nothing runnable
/// before the queue reaches the refused command: only a drain reaches it.
fn refusing_runtime() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(1), 100);
    for (imm, addr) in [(0xAA, 0), (0xBB, 8)] {
        rt.register_unit_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(imm),
                    FakeOp::SharedStore { addr, len: 4 },
                    FakeOp::End,
                ],
            )
        });
    }
    rt.register_unit_with(|id| RefusedCommandUnit {
        id,
        wait: 8,
        done: false,
    });
    rt
}

fn names_the_refusal(line: Option<&str>) {
    let line = line.expect("a run raised the exception");
    assert!(
        line.contains(&MfcCommandError::SizeUnaligned(3).to_string()),
        "{line}"
    );
}

#[test]
fn an_exploration_names_the_mfc_exception_its_runs_raised() {
    let result = explore(refusing_runtime, &ExplorationConfig::default()).expect("it branches");
    names_the_refusal(result.first_mfc_exception.as_deref());
}

#[test]
fn a_backtracking_exploration_names_it_too() {
    let result = explore_backtrack(refusing_runtime, &ExplorationConfig::default());
    names_the_refusal(result.first_mfc_exception.as_deref());
}
