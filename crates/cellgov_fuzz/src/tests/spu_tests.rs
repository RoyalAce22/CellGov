use super::*;
use crate::{ConfigurationError, RunOutcome};

#[test]
fn an_spu_sequence_retains_terminal_effects() {
    let mut initial = SpuState::new();
    let rd_in_mbox = (0x00d_u32 << 21) | (29 << 7) | 2;
    initial.ls[..4].copy_from_slice(&rd_in_mbox.to_be_bytes());

    let (observed, decoded, _) = run_sequence(&initial, 1);

    assert_eq!(decoded, 1);
    assert!(matches!(
        observed.terminal_outcome,
        Some(SpuStepOutcome::Yield { ref effects, .. }) if !effects.is_empty()
    ));
}

#[test]
fn a_faulting_spu_sequence_discards_prior_state() {
    let mut initial = SpuState::new();
    let il_r3_one = (0x081u32 << 23) | (1 << 7) | 3;
    let unsupported_rchcnt = (0x00fu32 << 21) | (8 << 7) | 2;
    initial.ls[..4].copy_from_slice(&il_r3_one.to_be_bytes());
    initial.ls[4..8].copy_from_slice(&unsupported_rchcnt.to_be_bytes());

    let (observed, decoded, _) = run_sequence(&initial, 2);

    assert_eq!(decoded, 2);
    assert_eq!(observed.state.regs[3], [0; 16]);
    assert_eq!(observed.state.pc, 0);
    assert!(matches!(
        observed.terminal_outcome,
        Some(SpuStepOutcome::Fault(_))
    ));
}

#[test]
fn self_modified_decode_refusal_is_a_terminal_observation() {
    let mut initial = SpuState::new();
    initial.regs[0] = [0xff; 16];
    let stqd_r0_at_16 = 0x2400_4080u32;
    let nop = 0x4020_007fu32;
    for (index, word) in [stqd_r0_at_16, nop, nop, nop, nop].iter().enumerate() {
        let start = index * 4;
        initial.ls[start..start + 4].copy_from_slice(&word.to_be_bytes());
    }

    let (observed, decoded, _) = run_sequence(&initial, 5);

    assert_eq!(decoded, 4);
    assert_eq!(observed.decode_refusal, Some((16, u32::MAX)));
}

#[test]
fn the_final_aligned_local_store_pc_is_generatable() {
    let final_slot = (SPU_LS_SIZE / 4 - 1) as u64;

    assert_eq!(pc_for_slot(final_slot), Ok((SPU_LS_SIZE - 4) as u32));
}

#[test]
fn a_zero_word_spu_sequence_is_an_invalid_configuration() {
    let run = run_sequences(FuzzConfig {
        iterations: 1,
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
fn unexpected_panics_keep_partial_report_evidence() {
    let config = FuzzConfig::default();
    let run = guarded_run(FuzzTarget::SpuInstruction, config, |report| {
        report.considered()?;
        record_target_panic(
            report,
            CheckIdentity::SpuDecoder,
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
            stage: "SPU campaign",
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
fn replay_requires_the_deterministic_relation() {
    assert!(!requests_replay(&[]));
    assert!(requests_replay(&[SpuMetamorphicRelation::Deterministic]));
}

#[test]
fn oversized_sequences_are_typed_refusals() {
    let run = run_sequences(FuzzConfig {
        iterations: 1,
        sequence_words: SPU_LS_SIZE / 4 + 1,
        ..FuzzConfig::default()
    });

    assert!(matches!(
        run.outcome,
        RunOutcome::HarnessFailure(FuzzError::Configuration(
            ConfigurationError::SequenceTooLong { .. }
        ))
    ));
}
