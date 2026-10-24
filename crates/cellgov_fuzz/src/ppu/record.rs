//! The findings an engine records when a contract check fails or the target
//! panics, and the guarded run that wraps every PPU engine.

use cellgov_ppu::instruction::fuzz::PpuGenerationDescriptor;

use crate::boundary::{call_harness, call_target};
use crate::error::{FuzzError, InvariantError};
use crate::registry::ppu_descriptors;
use crate::report::{
    CheckIdentity, DivergenceClass, Finding, FindingKind, FuzzReport, FuzzRun, FuzzTarget,
    InstructionIdentity, ReductionOutcome, SemanticFingerprint,
};
use crate::{FuzzConfig, GenerationStrategy, ReplayCoordinates, TargetPanicPayload};

/// The descriptor registry a campaign generates from: the registry for
/// a structured campaign with a first case, otherwise an empty list.
/// `None` when the registry panicked. The report then holds one
/// target-panic finding for `first_case`, and the campaign runs no case.
pub(super) fn load_descriptors(
    strategy: GenerationStrategy,
    first_case: Option<u64>,
    report: &mut FuzzReport,
) -> Result<Option<Vec<PpuGenerationDescriptor>>, InvariantError> {
    let (GenerationStrategy::Structured, Some(first)) = (strategy, first_case) else {
        return Ok(Some(Vec::new()));
    };
    match call_target(ppu_descriptors) {
        Ok(descriptors) => Ok(Some(descriptors)),
        Err(payload) => {
            report.considered()?;
            record_target_panic(
                report,
                CheckIdentity::PpuDecoder,
                None,
                Vec::new(),
                first,
                payload,
            )?;
            Ok(None)
        }
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
                stage: "PPU campaign",
            },
        ),
    }
}

#[cfg(test)]
#[path = "tests/record_tests.rs"]
mod tests;
