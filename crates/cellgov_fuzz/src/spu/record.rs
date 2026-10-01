//! The findings an engine records when a contract check fails or the target
//! panics, and the guarded run that wraps every SPU engine.

use cellgov_spu::observation::SpuObservationComponent;

use super::sequence_relations::divergence_class;
use crate::boundary::call_harness;
use crate::error::{FuzzError, InvariantError};
use crate::report::{
    CheckIdentity, ComponentIdentity, DivergenceClass, Finding, FindingKind, FuzzReport, FuzzRun,
    FuzzTarget, InstructionIdentity, OutcomeIdentity, ReductionOutcome, SemanticFingerprint,
};
use crate::{FuzzConfig, ReplayCoordinates, TargetPanicPayload};

/// The identity of a write outside the allowed footprint, in `component`;
/// see [`ComponentIdentity`] for why the component is part of it.
pub(super) fn footprint_fingerprint(
    target: FuzzTarget,
    instruction_kind: Option<InstructionIdentity>,
    component: SpuObservationComponent,
    outcome: Option<OutcomeIdentity>,
) -> SemanticFingerprint {
    SemanticFingerprint {
        target,
        instruction_kind,
        check: CheckIdentity::AllowedFootprint,
        divergence: divergence_class(component),
        outcome,
        effect: None,
        component: Some(ComponentIdentity::Spu(component)),
    }
}

pub(super) fn record(
    report: &mut FuzzReport,
    kind: FindingKind,
    fingerprint: SemanticFingerprint,
    original_words: Vec<u32>,
    iteration: u64,
) -> Result<(), InvariantError> {
    report.finding(Finding {
        fingerprint,
        kind,
        replay: ReplayCoordinates::new(
            report.target,
            report.strategy,
            report.seed,
            iteration,
            report.sequence_words,
        ),
        original_words,
        observation: None,
        reduction: ReductionOutcome::NotAttempted,
        panic_payload: None,
    })
}

pub(super) fn record_target_panic(
    report: &mut FuzzReport,
    check: CheckIdentity,
    instruction_kind: Option<InstructionIdentity>,
    original_words: Vec<u32>,
    iteration: u64,
    payload: TargetPanicPayload,
) -> Result<(), InvariantError> {
    report.finding(Finding {
        fingerprint: SemanticFingerprint {
            target: report.target,
            instruction_kind,
            check,
            divergence: DivergenceClass::TargetPanic,
            outcome: None,
            effect: None,
            component: None,
        },
        kind: FindingKind::TargetPanic,
        replay: ReplayCoordinates::new(
            report.target,
            report.strategy,
            report.seed,
            iteration,
            report.sequence_words,
        ),
        original_words,
        observation: None,
        reduction: ReductionOutcome::NotAttempted,
        panic_payload: Some(payload),
    })
}

pub(super) fn guarded_run(
    target: FuzzTarget,
    config: FuzzConfig,
    run: impl FnOnce(&mut FuzzReport) -> Result<(), FuzzError>,
) -> FuzzRun {
    let mut report = FuzzReport::new(
        target,
        config.seed,
        config.strategy,
        config.retention,
        config.max_findings as usize,
        config.sequence_words,
    );
    match call_harness(|| run(&mut report)) {
        Ok(Ok(())) if config.schedule.is_cancelled() && report.is_clean() => {
            FuzzRun::cancelled(report)
        }
        Ok(Ok(())) => FuzzRun::completed(report),
        Ok(Err(error)) => FuzzRun::failed(report, error),
        Err(_) => FuzzRun::failed(
            report,
            InvariantError::UnexpectedPanic {
                stage: "SPU campaign",
            },
        ),
    }
}

#[cfg(test)]
#[path = "tests/record_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/footprint_fingerprint_tests.rs"]
mod footprint_fingerprint_tests;
