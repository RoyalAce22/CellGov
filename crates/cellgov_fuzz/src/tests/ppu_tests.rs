use super::*;
use crate::{ConfigurationError, RunOutcome};

fn mftb(rt: u32) -> u32 {
    (31 << 26) | (rt << 21) | (12 << 16) | (8 << 11) | (371 << 1)
}

#[test]
fn seeded_vrsave_is_valid_for_an_immediate_read() {
    let mut rng = Rng::for_case(crate::CAMPAIGN_VERSION, 7, 0);
    let initial = random_state(&mut rng).unwrap();
    let observed = run_once(
        &PpuInstruction::Mfvrsave { rt: 0 },
        &initial,
        &[0; DATA_LEN],
    );

    assert_eq!(observed.state.gpr[0], u64::from(initial.vrsave));
}

#[test]
fn a_time_base_read_is_part_of_the_observed_effects() {
    let initial = PpuState::new();
    let instruction = cellgov_ppu::decode::decode(mftb(3)).unwrap();
    let observed = run_once(&instruction, &initial, &[0; DATA_LEN]);

    assert!(observed
        .effects
        .iter()
        .any(|effect| matches!(effect, Effect::ClockRead { source } if *source == UNIT)));
}

#[test]
fn a_ppu_sequence_retains_terminal_effects() {
    let initial = PpuState::new();
    let (observed, decoded, _) = run_sequence(&[mftb(3)], &initial, &[0; DATA_LEN]);

    assert_eq!(decoded, 1);
    assert!(matches!(
        observed.terminal_verdict,
        Some(ExecuteVerdict::Continue)
    ));
    assert!(observed
        .effects
        .iter()
        .any(|effect| matches!(effect, Effect::ClockRead { source } if *source == UNIT)));
}

#[test]
fn a_ppu_sequence_fetches_from_the_branch_target() {
    let branch_over_next_word = (18 << 26) | 8;
    let li_r3_one = (14 << 26) | (3 << 21) | 1;
    let nop = 24 << 26;
    let initial = PpuState::new();

    let (observed, decoded, _) = run_sequence(
        &[branch_over_next_word, li_r3_one, nop],
        &initial,
        &[0; DATA_LEN],
    );

    assert_eq!(decoded, 2);
    assert_eq!(observed.state.gpr[3], 0);
}

#[test]
fn a_faulting_ppu_sequence_discards_prior_state() {
    let li_r3_one = (14 << 26) | (3 << 21) | 1;
    let lwz_r4_r5 = (32 << 26) | (4 << 21) | (5 << 16);
    let initial = PpuState::new();

    let (observed, decoded, _) = run_sequence(&[li_r3_one, lwz_r4_r5], &initial, &[0; DATA_LEN]);

    assert_eq!(decoded, 2);
    assert_eq!(observed.state.gpr[3], 0);
    assert_eq!(observed.pc, 0);
    assert!(observed.effects.is_empty());
    assert!(matches!(
        observed.terminal_verdict,
        Some(ExecuteVerdict::MemFault(_))
    ));
}

#[test]
fn a_zero_word_ppu_sequence_is_an_invalid_configuration() {
    let run = run_sequences(FuzzConfig {
        sequence_words: 0,
        ..FuzzConfig::default()
    });

    assert!(matches!(
        run.outcome,
        RunOutcome::HarnessFailure(FuzzError::Configuration(
            ConfigurationError::ZeroSequenceWords
        ))
    ));
    assert_eq!(run.report.cases, 0);
}

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
fn replay_requires_the_deterministic_relation() {
    assert!(!requests_replay(&[]));
    assert!(requests_replay(&[PpuMetamorphicRelation::Deterministic]));
}

#[test]
fn panic_presentation_does_not_change_semantic_identity() {
    let mut left = FuzzReport::new(FuzzTarget::PpuInstruction, 7, 1, 1);
    let mut right = FuzzReport::new(FuzzTarget::PpuInstruction, 7, 1, 1);

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
