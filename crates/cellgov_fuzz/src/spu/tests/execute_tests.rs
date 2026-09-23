use super::*;
use cellgov_spu::fuzz::{generation_descriptors, SpuSequenceInteraction};
use cellgov_sync::{ReservedLine, RESERVATION_LINE_BYTES};

use crate::case::{CaseEligibility, EligibilityReason};
use crate::spu::assess::assess_sequence_case;
use crate::spu::generate::STRUCTURED_LS_DATA_BASE;

fn observed_step(outcome: SpuStepOutcome) -> ObservedStep {
    ObservedStep {
        outcome,
        state: SpuObservableSnapshot::capture(&SpuState::new()),
    }
}

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
fn replay_requires_the_deterministic_relation() {
    assert!(!requests_replay(&[]));
    assert!(requests_replay(&[SpuMetamorphicRelation::Deterministic]));
}

#[test]
fn spu_replay_asymmetry_uses_the_strongest_changed_axis() {
    let first = observed_step(SpuStepOutcome::Continue);

    assert_eq!(
        spu_outcome_class_asymmetry(SpuOutcomeClass::Continue),
        CrossReferenceAsymmetry::Outcome
    );
    assert_eq!(
        spu_outcome_class_asymmetry(SpuOutcomeClass::Fault),
        CrossReferenceAsymmetry::Fault
    );

    let mut second = first.clone();
    second.state.regs[0][0] = 1;
    assert_eq!(
        spu_step_replay_asymmetry(&first, &second),
        CrossReferenceAsymmetry::State
    );

    let second = observed_step(SpuStepOutcome::Branch);
    assert_eq!(
        spu_step_replay_asymmetry(&first, &second),
        CrossReferenceAsymmetry::Outcome
    );

    let second = observed_step(SpuStepOutcome::Fault(
        cellgov_spu::exec::SpuFault::UnsupportedChannelCount(0),
    ));
    assert_eq!(
        spu_step_replay_asymmetry(&first, &second),
        CrossReferenceAsymmetry::Fault
    );

    let rd_in_mbox = (0x00d_u32 << 21) | (29 << 7) | 2;
    let instruction = cellgov_spu::decode::decode(rd_in_mbox).unwrap();
    let first = run_once(&instruction, &SpuState::new());
    let mut second = first.clone();
    let SpuStepOutcome::Yield { effects, .. } = &mut second.outcome else {
        panic!("mailbox read must yield");
    };
    effects.push(Effect::ClockRead { source: UNIT });
    assert_eq!(
        spu_step_replay_asymmetry(&first, &second),
        CrossReferenceAsymmetry::Effect
    );
}

#[test]
fn spu_sequence_replay_compares_depth_and_execution_order() {
    let observed = ObservedSequence {
        state: SpuObservableSnapshot::capture(&SpuState::new()),
        terminal_outcome: Some(SpuStepOutcome::Continue),
        decode_refusal: None,
        deterministic: true,
        has_undefined_operands: false,
        has_unmodeled_execution: false,
        footprint_violations: BTreeSet::new(),
    };
    let nop = InstructionIdentity::Spu(cellgov_spu::instruction::SpuInstructionKind::Nop);
    let lnop = InstructionIdentity::Spu(cellgov_spu::instruction::SpuInstructionKind::Lnop);
    let first = (observed.clone(), 2, vec![nop, lnop]);
    let reordered = (observed.clone(), 2, vec![lnop, nop]);
    let deeper = (observed, 3, vec![nop, lnop]);
    let mut refusal = first.clone();
    refusal.0.decode_refusal = Some((8, u32::MAX));

    assert_eq!(
        spu_sequence_replay_asymmetry(&first, &reordered),
        CrossReferenceAsymmetry::State
    );
    assert_eq!(
        spu_sequence_replay_asymmetry(&first, &deeper),
        CrossReferenceAsymmetry::State
    );
    assert_eq!(
        spu_sequence_replay_asymmetry(&first, &refusal),
        CrossReferenceAsymmetry::Outcome
    );
}

#[test]
fn spu_observation_preserves_the_first_executed_kind() {
    let nop = InstructionIdentity::Spu(cellgov_spu::instruction::SpuInstructionKind::Nop);
    let lnop = InstructionIdentity::Spu(cellgov_spu::instruction::SpuInstructionKind::Lnop);
    let assessment = CaseAssessment::new(
        CaseEligibility::Eligible,
        EligibilityReason::InterpreterContract,
        BTreeSet::new(),
    );
    let state = SpuObservableSnapshot::capture(&SpuState::new());

    let observation = spu_observation(
        [lnop, nop],
        &assessment,
        SpuTerminalObservation::Execution(Some(&SpuStepOutcome::Continue)),
        &state,
        state.clone(),
        2,
        CrossReferenceAsymmetry::None,
    );

    assert_eq!(observation.first_instruction_kind, Some(lnop));

    let refusal = spu_observation(
        [lnop, nop],
        &assessment,
        SpuTerminalObservation::DecodeRefusal(Some(&SpuStepOutcome::Continue)),
        &state,
        state.clone(),
        2,
        CrossReferenceAsymmetry::None,
    );
    assert_eq!(refusal.outcome, Some(OutcomeIdentity::SpuDecodeRefusal));
}

#[test]
fn every_spu_interaction_recipe_executes_its_dependency_and_detects_seeded_leaks() {
    let descriptors = generation_descriptors();
    for interaction in SpuSequenceInteraction::ALL {
        let words = interaction
            .words(&descriptors)
            .expect("each interaction must encode");
        let mut initial = SpuState::new();
        initial.channels.mfc_size = RESERVATION_LINE_BYTES as u32;
        initial.channels.mfc_eal = STRUCTURED_LS_DATA_BASE;
        initial.channels.tag_mask = 1;
        initial.channels.tag_status = 1;
        initial.reservation = Some(ReservedLine::containing(u64::from(STRUCTURED_LS_DATA_BASE)));
        interaction.prepare_state(&mut initial, STRUCTURED_LS_DATA_BASE);
        if matches!(
            interaction,
            SpuSequenceInteraction::Channel
                | SpuSequenceInteraction::Dma
                | SpuSequenceInteraction::DmaGet
                | SpuSequenceInteraction::MemoryRead
                | SpuSequenceInteraction::Reservation
        ) {
            assert_ne!(initial.channels.mfc_lsa, STRUCTURED_LS_DATA_BASE);
        }
        for (index, word) in words.iter().enumerate() {
            initial.ls[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        let first = run_generated_sequence(
            &initial,
            words.len(),
            words.len(),
            GenerationStrategy::Structured,
            &Cell::new(None),
        );
        let replay = run_generated_sequence(
            &initial,
            words.len(),
            words.len(),
            GenerationStrategy::Structured,
            &Cell::new(None),
        );
        assert_eq!(
            spu_sequence_replay_asymmetry(&first, &replay),
            CrossReferenceAsymmetry::None,
            "{interaction:?}"
        );
        let intended = words
            .iter()
            .map(|word| {
                let decoded = cellgov_spu::decode::decode(*word).expect("recipe word must decode");
                InstructionIdentity::Spu(decoded.fuzz_descriptor().kind)
            })
            .collect::<Vec<_>>();
        assert_eq!(first.2.first(), intended.first(), "{interaction:?}");
        assert!(
            first.1 >= 2 || interaction == SpuSequenceInteraction::Branch,
            "{interaction:?}"
        );
        assert!(first.1 <= words.len() as u64, "{interaction:?}");
        assert!(first.0.decode_refusal.is_none(), "{interaction:?}");
        match interaction {
            SpuSequenceInteraction::Channel => {
                assert_eq!(first.2.len(), 3);
                assert_eq!(first.0.state.channels.mfc_lsa, STRUCTURED_LS_DATA_BASE);
                assert!(matches!(
                    first.0.terminal_outcome,
                    Some(SpuStepOutcome::Continue)
                ));
            }
            SpuSequenceInteraction::Mailbox => {
                assert_eq!(first.2.len(), 2);
                assert_eq!(first.0.state.channels.pending_mbox_rt, Some(3));
                assert!(matches!(
                    first.0.terminal_outcome,
                    Some(SpuStepOutcome::Yield { .. })
                ));
            }
            SpuSequenceInteraction::Dma
            | SpuSequenceInteraction::DmaGet
            | SpuSequenceInteraction::Reservation => {
                assert_eq!(first.2.len(), 2);
                assert!(
                    matches!(first.0.terminal_outcome, Some(SpuStepOutcome::Yield { .. })),
                    "{interaction:?}: {:?}",
                    first.0.terminal_outcome
                );
                if interaction == SpuSequenceInteraction::Reservation {
                    assert!(first.0.state.reservation.is_none());
                } else if interaction == SpuSequenceInteraction::DmaGet {
                    assert!(first.0.state.channels.pending_get.is_some());
                }
            }
            SpuSequenceInteraction::MemoryRead => {
                assert_eq!(first.2.len(), 2);
                assert!(matches!(
                    first.0.terminal_outcome,
                    Some(SpuStepOutcome::MemoryRead { .. })
                ));
            }
            SpuSequenceInteraction::LocalStore => {
                assert_eq!(first.2.len(), 2);
                assert_eq!(
                    first.0.state.regs[3],
                    [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
                );
            }
            SpuSequenceInteraction::LocalStoreFault => {
                assert_eq!(first.2.len(), 2);
                assert!(matches!(
                    first.0.terminal_outcome,
                    Some(SpuStepOutcome::Fault(_))
                ));
                assert_eq!(first.0.state, SpuObservableSnapshot::capture(&initial));
                assert!(first.0.footprint_violations.is_empty());
            }
            SpuSequenceInteraction::Branch => {
                assert_eq!(first.2.len(), 2);
                assert_eq!(first.0.state.pc, 12);
                assert_eq!(first.0.state.regs[5], initial.regs[5], "decoy must not run");
                assert_eq!(
                    first.2[1],
                    InstructionIdentity::Spu(cellgov_spu::instruction::SpuInstructionKind::Nop)
                );
            }
            SpuSequenceInteraction::Stop => {
                assert_eq!(first.2.len(), 2);
                assert!(first.0.has_unmodeled_execution);
                // STOP raises an external signal the campaign does not model, so the case is unsupported.
                assert_eq!(
                    assess_sequence_case(
                        GenerationStrategy::Structured,
                        first.1,
                        false,
                        first.0.has_unmodeled_execution,
                        BTreeSet::new()
                    )
                    .eligibility,
                    CaseEligibility::Unsupported
                );
                assert!(matches!(
                    first.0.terminal_outcome,
                    Some(SpuStepOutcome::Yield { .. })
                ));
            }
        }
        let mut defective = replay;
        match interaction {
            SpuSequenceInteraction::Channel => defective.0.state.channels.mfc_lsa ^= 16,
            SpuSequenceInteraction::Mailbox => defective.0.state.channels.pending_mbox_rt = None,
            SpuSequenceInteraction::Dma => {
                if let Some(SpuStepOutcome::Yield { effects, .. }) =
                    &mut defective.0.terminal_outcome
                {
                    effects.clear();
                }
            }
            SpuSequenceInteraction::DmaGet => defective.0.state.channels.pending_get = None,
            SpuSequenceInteraction::MemoryRead => {
                defective.0.terminal_outcome = Some(SpuStepOutcome::Continue)
            }
            SpuSequenceInteraction::Reservation => {
                defective.0.state.reservation = initial.reservation
            }
            SpuSequenceInteraction::LocalStore => {
                defective.0.state.ls[STRUCTURED_LS_DATA_BASE as usize] ^= 1
            }
            SpuSequenceInteraction::LocalStoreFault => defective.0.state.pc ^= 4,
            SpuSequenceInteraction::Branch => defective.0.state.pc ^= 4,
            SpuSequenceInteraction::Stop => {
                defective.0.terminal_outcome = Some(SpuStepOutcome::Continue)
            }
        }
        assert_ne!(
            spu_sequence_replay_asymmetry(&first, &defective),
            CrossReferenceAsymmetry::None,
            "{interaction:?}"
        );
    }
}

#[test]
fn an_spu_sequence_does_not_execute_beyond_its_generated_words() {
    let mut initial = SpuState::new();
    let branch = generation_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.kind == cellgov_spu::instruction::SpuInstructionKind::Br)
        .expect("branch generation descriptor must exist");
    let mut parameters = branch.canonical_parameters();
    parameters[0] = 2;
    let branch_to_third_word = branch
        .encode(&parameters)
        .expect("branch to third word must encode");
    initial.ls[..4].copy_from_slice(&branch_to_third_word.to_be_bytes());
    initial.ls[8..12].copy_from_slice(&branch.canonical_word.to_be_bytes());

    let (_, decoded, _) = run_generated_sequence(
        &initial,
        2,
        2,
        GenerationStrategy::Structured,
        &Cell::new(None),
    );

    assert_eq!(decoded, 1);
}
