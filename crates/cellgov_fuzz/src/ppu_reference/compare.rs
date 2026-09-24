//! Offline replay of a PPU reference and the comparison against each execution path.

use std::collections::BTreeSet;

use cellgov_effects::Effect;
use cellgov_exec::YieldReason;
use cellgov_ppu::observation::PpuArchitecturalState;

use crate::ppu_paths::{first_path_divergence, run_all_paths, PpuPathRun};
use crate::reference::{compare_field, ReferenceComparison, ReferenceField, ReferenceOmission};

use super::types::{
    PpuReferenceArtifact, PpuReferenceComparison, PpuReferenceComponent, PpuReferenceDifference,
    PpuReferenceError, PpuReferenceFault, PpuReferenceFaultRegisters, PpuReferenceObservation,
    PpuReferenceReplay, PpuReferenceState, PpuReferenceYieldReason, PpuUnrepresentedField,
};

/// Replays one artifact without hardware, a network, or an external executable.
// [Martignoni2009 p:127 s:2.3] Both CPUs start from the same synthetic state and execute the case; the comparison reads only their final states.
pub fn replay_reference(
    artifact: &PpuReferenceArtifact,
) -> Result<PpuReferenceReplay, PpuReferenceError> {
    artifact.validate()?;
    let initial = artifact.initial_state.to_state()?;
    let runs = run_all_paths(&artifact.words, &initial, &artifact.initial_memory)?;
    let internal_divergence = first_path_divergence(&runs);
    let comparisons = runs
        .iter()
        .map(|run| compare_reference(&artifact.expected, run))
        .collect();
    Ok(PpuReferenceReplay {
        runs,
        internal_divergence,
        comparisons,
    })
}

/// Compares only fields represented by both the reference and CellGov.
// [Watt2023 p:110:2 s:1] A reference earns its trust from its proven correspondence to the specification, independent of the implementation it checks.
// [Jiang2022 p:5 s:3.2.1] The compared final state is the program counter, the registers, only the memory the case can write, the status bits, and the signal raised.
pub fn compare_reference(
    expected: &PpuReferenceObservation,
    run: &PpuPathRun,
) -> PpuReferenceComparison {
    let mut comparison = PpuReferenceComparison {
        compared: BTreeSet::new(),
        differences: Vec::new(),
        unrepresented: Vec::new(),
    };
    compare_state(&expected.state, &run.observation.state, &mut comparison);
    compare_value(
        PpuReferenceComponent::Memory,
        &expected.memory,
        &run.observation.memory,
        &mut comparison,
    );
    compare_value(
        PpuReferenceComponent::StopReason,
        &expected.stop.reason,
        &reference_yield(run.stop.reason),
        &mut comparison,
    );
    let fault = match run.stop.fault.as_ref() {
        None => PpuReferenceFault::None,
        Some(cellgov_effects::FaultKind::Validation) => PpuReferenceFault::Validation,
        Some(cellgov_effects::FaultKind::Guest(code)) => PpuReferenceFault::Guest { code: *code },
    };
    compare_value(
        PpuReferenceComponent::StopFault,
        &expected.stop.fault,
        &fault,
        &mut comparison,
    );
    compare_value(
        PpuReferenceComponent::StopPc,
        &expected.stop.pc,
        &run.stop.pc,
        &mut comparison,
    );
    compare_value(
        PpuReferenceComponent::StopLr,
        &expected.stop.lr,
        &run.stop.diagnostics.lr,
        &mut comparison,
    );
    compare_value(
        PpuReferenceComponent::StopSyscallLev,
        &expected.stop.syscall_lev,
        &run.stop.diagnostics.syscall_lev,
        &mut comparison,
    );
    compare_value(
        PpuReferenceComponent::StopFaultingEa,
        &expected.stop.faulting_ea,
        &run.stop.diagnostics.faulting_ea,
        &mut comparison,
    );
    let fault_registers =
        run.stop
            .diagnostics
            .fault_regs
            .as_ref()
            .map(|registers| PpuReferenceFaultRegisters {
                gpr: registers.gprs.to_vec(),
                lr: registers.lr,
                ctr: registers.ctr,
                xer: registers.xer,
                cr: registers.cr,
            });
    compare_value(
        PpuReferenceComponent::StopFaultRegisters,
        &expected.stop.fault_registers,
        &fault_registers,
        &mut comparison,
    );
    compare_value(
        PpuReferenceComponent::StopSyscallArgs,
        &expected.stop.syscall_args,
        &run.stop.syscall_args.map(|args| args.to_vec()),
        &mut comparison,
    );
    compare_value(
        PpuReferenceComponent::Retired,
        &expected.retired,
        &run.retired,
        &mut comparison,
    );
    compare_value(
        PpuReferenceComponent::StagedEffects,
        &expected.staged_effects,
        &render_effects(&run.observation.staged_effects),
        &mut comparison,
    );
    compare_value(
        PpuReferenceComponent::CommittedEffects,
        &expected.committed_effects,
        &render_effects(&run.observation.committed_effects),
        &mut comparison,
    );
    let reservations = run
        .observation
        .reservations
        .iter()
        .map(|(unit, line)| (unit.raw(), line.addr()))
        .collect::<Vec<_>>();
    compare_value(
        PpuReferenceComponent::Reservations,
        &expected.reservations,
        &reservations,
        &mut comparison,
    );
    let stores = run
        .observation
        .store_buffer
        .iter()
        .map(|store| format!("{store:?}"))
        .collect::<Vec<_>>();
    compare_value(
        PpuReferenceComponent::StoreBuffer,
        &expected.store_buffer,
        &stores,
        &mut comparison,
    );
    let commit_error = run
        .observation
        .commit_error
        .as_ref()
        .map(ToString::to_string);
    compare_value(
        PpuReferenceComponent::CommitError,
        &expected.commit_error,
        &commit_error,
        &mut comparison,
    );
    compare_value(
        PpuReferenceComponent::FaultDiscarded,
        &expected.fault_discarded,
        &run.observation.fault_discarded,
        &mut comparison,
    );
    comparison
}

fn compare_state(
    expected: &PpuReferenceState,
    observed: &PpuArchitecturalState,
    comparison: &mut PpuReferenceComparison,
) {
    compare_value(
        PpuReferenceComponent::StateGpr,
        &expected.gpr,
        &observed.gpr.to_vec(),
        comparison,
    );
    compare_value(
        PpuReferenceComponent::StateFpr,
        &expected.fpr,
        &observed.fpr.to_vec(),
        comparison,
    );
    let vr_hex = observed
        .vr
        .iter()
        .map(|value| format!("{value:032x}"))
        .collect::<Vec<_>>();
    compare_value(
        PpuReferenceComponent::StateVr,
        &expected.vr_hex,
        &vr_hex,
        comparison,
    );
    compare_value(
        PpuReferenceComponent::StatePc,
        &expected.pc,
        &observed.pc,
        comparison,
    );
    compare_value(
        PpuReferenceComponent::StateCr,
        &expected.cr,
        &observed.cr,
        comparison,
    );
    compare_value(
        PpuReferenceComponent::StateLr,
        &expected.lr,
        &observed.lr,
        comparison,
    );
    compare_value(
        PpuReferenceComponent::StateCtr,
        &expected.ctr,
        &observed.ctr,
        comparison,
    );
    compare_value(
        PpuReferenceComponent::StateXer,
        &expected.xer,
        &observed.xer,
        comparison,
    );
    compare_value(
        PpuReferenceComponent::StateVrsave,
        &expected.vrsave,
        &observed.vrsave,
        comparison,
    );
    compare_value(
        PpuReferenceComponent::StateTb,
        &expected.tb,
        &observed.tb,
        comparison,
    );
    let reservation = observed.reservation.map(|line| line.addr());
    compare_value(
        PpuReferenceComponent::StateReservation,
        &expected.reservation,
        &reservation,
        comparison,
    );
}

/// Compares a field by value, recording both sides' debug renderings
/// on a disagreement.
fn compare_value<T: std::fmt::Debug + PartialEq>(
    field: PpuReferenceComponent,
    expected: &ReferenceField<T>,
    observed: &T,
    comparison: &mut PpuReferenceComparison,
) {
    compare_field(comparison, field, expected, |value| {
        (value != observed).then(|| (format!("{value:?}"), format!("{observed:?}")))
    });
}

impl ReferenceComparison for PpuReferenceComparison {
    type Component = PpuReferenceComponent;
    type Difference = (String, String);

    fn compared(&mut self, component: PpuReferenceComponent) {
        self.compared.insert(component);
    }

    fn differs(
        &mut self,
        component: PpuReferenceComponent,
        (expected, observed): (String, String),
    ) {
        self.differences.push(PpuReferenceDifference {
            field: component,
            expected,
            observed,
        });
    }

    fn omitted(
        &mut self,
        component: PpuReferenceComponent,
        omission: ReferenceOmission,
        reason: &str,
    ) {
        self.unrepresented.push(PpuUnrepresentedField {
            field: component,
            status: omission,
            reason: reason.to_owned(),
        });
    }
}

pub(super) fn reference_yield(reason: YieldReason) -> PpuReferenceYieldReason {
    match reason {
        YieldReason::BudgetExhausted => PpuReferenceYieldReason::BudgetExhausted,
        YieldReason::MailboxAccess => PpuReferenceYieldReason::MailboxAccess,
        YieldReason::DmaSubmitted => PpuReferenceYieldReason::DmaSubmitted,
        YieldReason::DmaWait => PpuReferenceYieldReason::DmaWait,
        YieldReason::WaitingSync => PpuReferenceYieldReason::WaitingSync,
        YieldReason::Syscall => PpuReferenceYieldReason::Syscall,
        YieldReason::InterruptBoundary => PpuReferenceYieldReason::InterruptBoundary,
        YieldReason::Fault => PpuReferenceYieldReason::Fault,
        YieldReason::Finished => PpuReferenceYieldReason::Finished,
    }
}

pub(super) fn render_effects(effects: &[Effect]) -> Vec<String> {
    effects.iter().map(|effect| format!("{effect:?}")).collect()
}
