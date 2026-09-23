use super::*;
use crate::{GenerationStrategy, RetentionConfig, RunOutcome};

#[test]
fn typed_failures_keep_partial_report_evidence() {
    let config = FuzzConfig::default();
    let run = guarded_run(FuzzTarget::PpuInstruction, config, |report| {
        report.considered()?;
        Err(InvariantError::CounterOverflow {
            counter: "test counter",
        }
        .into())
    });

    assert!(matches!(
        run.outcome,
        RunOutcome::HarnessFailure(FuzzError::Invariant(InvariantError::CounterOverflow {
            counter: "test counter",
        }))
    ));
    assert_eq!(run.report.cases, 1);
}

#[test]
fn unexpected_panics_keep_partial_report_evidence() {
    let config = FuzzConfig::default();
    let run = guarded_run(FuzzTarget::PpuInstruction, config, |report| {
        report.considered()?;
        record_target_panic(
            report,
            CheckIdentity::PpuDecoder,
            None,
            vec![0],
            0,
            TargetPanicPayload::NonString,
        )?;
        panic!("test harness panic");
    });

    assert!(matches!(
        run.outcome,
        RunOutcome::HarnessFailure(FuzzError::Invariant(InvariantError::UnexpectedPanic {
            stage: "PPU campaign",
        }))
    ));
    assert_eq!(run.report.cases, 1);
    assert_eq!(
        run.report.finding_counts.get(&FindingKind::TargetPanic),
        Some(&1)
    );
    assert_eq!(run.report.findings.len(), 1);
}

#[test]
fn cancellation_does_not_hide_a_target_panic() {
    let config = FuzzConfig {
        schedule: crate::CampaignSchedule {
            cancellation: Some(crate::CancellationBoundary(1)),
            ..crate::CampaignSchedule::default()
        },
        ..FuzzConfig::default()
    };
    let run = guarded_run(FuzzTarget::PpuInstruction, config, |report| {
        report.considered()?;
        record_target_panic(
            report,
            CheckIdentity::PpuDecoder,
            None,
            vec![0],
            0,
            TargetPanicPayload::NonString,
        )?;
        Ok(())
    });

    assert_eq!(run.outcome, RunOutcome::TargetPanic);
    assert_eq!(run.report.cases, 1);
    assert_eq!(run.report.findings.len(), 1);
    assert_eq!(
        run.report.findings[0].replay.target,
        FuzzTarget::PpuInstruction
    );
}

#[test]
fn panic_presentation_does_not_change_semantic_identity() {
    let mut left = FuzzReport::new(
        FuzzTarget::PpuInstruction,
        7,
        GenerationStrategy::Structured,
        RetentionConfig::default(),
        1,
        1,
    );
    let mut right = FuzzReport::new(
        FuzzTarget::PpuInstruction,
        7,
        GenerationStrategy::Structured,
        RetentionConfig::default(),
        1,
        1,
    );

    record_target_panic(
        &mut left,
        CheckIdentity::PpuDecoder,
        None,
        vec![0],
        3,
        TargetPanicPayload::StaticStr("first wording".to_owned()),
    )
    .unwrap();
    record_target_panic(
        &mut right,
        CheckIdentity::PpuDecoder,
        None,
        vec![0],
        3,
        TargetPanicPayload::String("second wording".to_owned()),
    )
    .unwrap();

    assert_eq!(left.findings[0].fingerprint, right.findings[0].fingerprint);
    assert_ne!(
        left.findings[0].panic_payload,
        right.findings[0].panic_payload
    );
}
