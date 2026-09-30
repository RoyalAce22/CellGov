//! Scenario runner outcomes -- stall vs max-steps, trace contents, and byte-identical reruns.

use super::*;
use crate::world::CountingUnit;
use cellgov_core::Runtime;
use cellgov_dma::{InvalidMfcCommand, MfcCommandError, MfcParameters};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_time::{Budget, InstructionCost};
use cellgov_trace::{TraceReader, TraceRecord};

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
fn a_result_names_the_mfc_exception_its_run_stalled_on() {
    let clean = run(ScenarioFixture::empty());
    assert_eq!(clean.mfc_exception, None);

    let refused = run(ScenarioFixture::builder()
        .memory_size(16)
        .max_steps(100)
        .register(|rt: &mut Runtime| {
            rt.register_unit_with(|id| RefusedCommandUnit { id, done: false });
        })
        .build());
    assert_eq!(refused.outcome, ScenarioOutcome::Stalled);
    let exception = refused.mfc_exception.expect("the run raised the command");
    assert_eq!(exception.unit, UnitId::new(0));
    assert_eq!(exception.command, REFUSED);
}

#[test]
fn a_result_carries_the_run_s_first_invariant_break_for_its_driver_to_report() {
    let clean = run(ScenarioFixture::empty());
    assert_eq!(
        clean.first_invariant_break, None,
        "a run that broke no invariant gives its driver no line to report"
    );

    let broken = run(ScenarioFixture::builder()
        .register(|rt: &mut Runtime| {
            rt.lv2_host_mut()
                .log_invariant_break("test.site", format_args!("details here"));
        })
        .build());
    assert_eq!(
        broken.first_invariant_break.as_deref(),
        Some("lv2 host invariant break at test.site: details here (the first of 1)"),
        "the runtime ends inside the runner, so a break it recorded reaches a driver \
         only through this field"
    );
}

#[test]
fn empty_fixture_stalls_immediately_with_no_steps() {
    let result = run(ScenarioFixture::empty());
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    assert_eq!(result.steps_taken, 0);
    assert!(result.trace_bytes.is_empty());
}

#[test]
fn single_unit_runs_to_completion_then_stalls() {
    let result = run(ScenarioFixture::builder()
        .memory_size(16)
        .budget(Budget::new(1))
        .max_steps(100)
        .register(|rt: &mut Runtime| {
            rt.register_unit_with(|id| CountingUnit::new(id, 5));
        })
        .build());
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    assert_eq!(result.steps_taken, 5);
    let scheduled_count = TraceReader::new(&result.trace_bytes)
        .map(|r| r.expect("decode"))
        .filter(|r| matches!(r, TraceRecord::UnitScheduled { .. }))
        .count();
    assert_eq!(scheduled_count, 5);
}

#[test]
fn max_steps_cap_surfaces_as_max_steps_exceeded() {
    let result = run(ScenarioFixture::builder()
        .memory_size(16)
        .budget(Budget::new(1))
        .max_steps(3)
        .register(|rt: &mut Runtime| {
            rt.register_unit_with(|id| CountingUnit::new(id, u64::MAX));
        })
        .build());
    assert_eq!(result.outcome, ScenarioOutcome::MaxStepsExceeded);
    assert_eq!(result.steps_taken, 3);
}

#[test]
fn two_runs_of_the_same_fixture_are_byte_identical() {
    fn build_and_run() -> ScenarioResult {
        run(ScenarioFixture::builder()
            .memory_size(16)
            .budget(Budget::new(2))
            .max_steps(100)
            .register(|rt: &mut Runtime| {
                rt.register_unit_with(|id| CountingUnit::new(id, 4));
                rt.register_unit_with(|id| CountingUnit::new(id, 6));
            })
            .build())
    }
    let a = build_and_run();
    let b = build_and_run();
    assert_eq!(a.outcome, b.outcome);
    assert_eq!(a.steps_taken, b.steps_taken);
    assert_eq!(a.trace_bytes, b.trace_bytes);
    assert_eq!(a.final_memory_hash, b.final_memory_hash);
    assert_eq!(a.final_unit_status_hash, b.final_unit_status_hash);
}
