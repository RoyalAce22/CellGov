use super::*;

#[test]
fn structured_generation_selects_every_interaction_with_bounded_linked_parameters() {
    let descriptors = generation_descriptors();
    let mut reached = BTreeSet::new();
    for case in 0..4_096 {
        let mut rng = Rng::for_case(FuzzConfig::default().campaign_version, 7, case);
        let generated =
            structured_sequence(&descriptors, &mut rng, 4).expect("typed sequence must generate");
        assert_eq!(generated.words.len(), 4);
        if let Some((interaction, data_base)) = generated.interaction {
            reached.insert(interaction);
            assert!(data_base >= STRUCTURED_LS_DATA_BASE);
            assert!(data_base as usize + RESERVATION_LINE_BYTES as usize <= SPU_LS_SIZE);
            assert_eq!(data_base as usize % RESERVATION_LINE_BYTES as usize, 0);
            let shorter = interaction
                .shrink_words(&generated.words)
                .expect("one trailing decoy is removable");
            assert_eq!(shorter.len(), 3);
            assert_eq!(&shorter[..], &generated.words[..3]);
            if matches!(
                interaction,
                SpuSequenceInteraction::Branch | SpuSequenceInteraction::Channel
            ) {
                assert!(
                    interaction.shrink_words(&shorter).is_none(),
                    "branch target is protected"
                );
            } else {
                assert_eq!(
                    interaction
                        .shrink_words(&shorter)
                        .expect("second decoy is removable")
                        .len(),
                    2
                );
            }
        }
    }
    assert_eq!(reached, BTreeSet::from(SpuSequenceInteraction::ALL));
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
        let first = run_generated_sequence(&initial, words.len(), GenerationStrategy::Structured);
        let replay = run_generated_sequence(&initial, words.len(), GenerationStrategy::Structured);
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
                // The campaign does not model STOP's external signal, so it refuses comparison.
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

    let (_, decoded, _) = run_generated_sequence(&initial, 2, GenerationStrategy::Structured);

    assert_eq!(decoded, 1);
}
