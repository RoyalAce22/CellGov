use super::*;
use cellgov_spu::fuzz::generation_descriptors;

#[test]
fn the_final_aligned_local_store_pc_is_generatable() {
    let final_slot = (SPU_LS_SIZE / 4 - 1) as u64;

    assert_eq!(pc_for_slot(final_slot), Ok((SPU_LS_SIZE - 4) as u32));
}

#[test]
fn declared_state_input_replaces_the_selected_register_word() {
    const EXPECTED: &[u32] = &[3, 5, 8];

    let mut descriptor = cellgov_spu::instruction::SpuInstruction::Ai {
        rt: 0,
        ra: 1,
        imm: 0,
    }
    .fuzz_descriptor();
    descriptor.state_input = Some(SpuStateInput {
        register: 7,
        values: EXPECTED,
        preferred: None,
    });
    let config = FuzzConfig::default();
    let mut rng = Rng::for_case(config.campaign_version, config.seed, 0);

    let (state, _) = state_aware_state(&mut rng, descriptor.state_input)
        .expect("declared state inputs must construct state");

    assert!(EXPECTED.contains(&state.reg_word(7)));
}

#[test]
fn structured_generation_selects_every_interaction_with_bounded_linked_parameters() {
    const CASES: u64 = 4_096;
    const WORDS: usize = 6;
    let descriptors = generation_descriptors();
    let mut reached = BTreeSet::new();
    let mut facilities = 0;
    for case in 0..CASES {
        let mut rng = Rng::for_case(FuzzConfig::default().campaign_version, 7, case);
        let generated = structured_sequence(&descriptors, &mut rng, WORDS)
            .expect("typed sequence must generate");
        assert_eq!(generated.words.len(), WORDS);
        if let Some((interaction, data_base)) = generated.interaction {
            reached.insert(interaction);
            facilities += u64::from(SpuSequenceInteraction::FACILITIES.contains(&interaction));
            assert!(data_base >= STRUCTURED_LS_DATA_BASE);
            assert!(data_base as usize + RESERVATION_LINE_BYTES as usize <= SPU_LS_SIZE);
            assert_eq!(data_base as usize % RESERVATION_LINE_BYTES as usize, 0);
            // Shrinking removes the trailing decoys one at a time and stops
            // at the interaction's own program.
            let recipe = interaction.words(&descriptors).expect("the recipe encodes");
            let mut words = generated.words.clone();
            while let Some(shorter) = interaction.shrink_words(&words) {
                assert_eq!(shorter.len() + 1, words.len());
                words = shorter;
            }
            assert_eq!(words, recipe, "{interaction:?}");
        }
    }
    assert_eq!(reached, BTreeSet::from(SpuSequenceInteraction::ALL));
    // Half the cases stage an interaction, and half of those draw from the
    // facility patterns alone, so more than a quarter of the cases wait on
    // a facility.
    assert!(facilities > CASES / 4, "{facilities} facility cases");
}

#[test]
fn a_sequence_too_short_for_a_facility_pattern_draws_one_that_fits() {
    let descriptors = generation_descriptors();
    for case in 0..1_024 {
        let mut rng = Rng::for_case(FuzzConfig::default().campaign_version, 7, case);
        let generated =
            structured_sequence(&descriptors, &mut rng, 3).expect("typed sequence must generate");
        if let Some((interaction, _)) = generated.interaction {
            let recipe = interaction.words(&descriptors).expect("the recipe encodes");
            assert!(recipe.len() <= 3, "{interaction:?}");
            assert_eq!(generated.words[..recipe.len()], recipe[..]);
        }
    }
}

#[test]
fn a_facility_world_carries_the_operations_its_pattern_waits_on() {
    let config = FuzzConfig::default();
    let mut rng = Rng::for_case(config.campaign_version, 3, 0);
    let world = sequence_world(&mut rng, Some(SpuSequenceInteraction::MailboxDrain), 4)
        .expect("the world draws");
    let ops = world
        .actions
        .iter()
        .map(|(_, action)| *action)
        .collect::<Vec<_>>();
    let read = ops
        .iter()
        .position(|action| *action == SpuReferencePpuAction::ReadOutMbox)
        .expect("the drain's outbound read is scheduled");
    let message = ops
        .iter()
        .position(|action| matches!(action, SpuReferencePpuAction::InMbox { .. }))
        .expect("the drain's inbound message is scheduled");
    assert!(read < message);
    assert!(world.actions.windows(2).all(|pair| pair[0].0 <= pair[1].0));
    assert_eq!(world.memory.len(), 1);
    assert_eq!(world.memory[0].0, u64::from(STRUCTURED_LS_DATA_BASE));
    assert_eq!(world.memory[0].1.len(), SEQUENCE_MAIN_STORAGE_BYTES);

    let mut rng = Rng::for_case(config.campaign_version, 3, 0);
    let world = sequence_world(&mut rng, Some(SpuSequenceInteraction::EventWait), 3)
        .expect("the world draws");
    assert!(world
        .actions
        .iter()
        .any(|(_, action)| matches!(action, SpuReferencePpuAction::Signal { register: 1, .. })));
}

#[test]
fn a_facility_world_lands_each_scripted_operation_after_the_access_that_waits_on_it() {
    use cellgov_spu::instruction::SpuInstruction;
    let descriptors = generation_descriptors();
    // The words that block until the PPU acts: the event-status read, and
    // the write to a full outbound mailbox and the read of a drained
    // inbound one.
    for (interaction, waits) in [
        (SpuSequenceInteraction::EventWait, &[1u64][..]),
        (SpuSequenceInteraction::MailboxDrain, &[1, 3][..]),
    ] {
        let recipe = interaction.words(&descriptors).expect("the recipe encodes");
        for wait in waits {
            assert!(matches!(
                cellgov_spu::decode::decode(recipe[*wait as usize]),
                Ok(SpuInstruction::Rdch { .. } | SpuInstruction::Wrch { .. })
            ));
        }
        let mut rng = Rng::for_case(FuzzConfig::default().campaign_version, 3, 0);
        // With no words to land after, every drawn operation lands first,
        // so the scripted ones are those that land later.
        let world = sequence_world(&mut rng, Some(interaction), 0).expect("the world draws");
        let scripted = world
            .actions
            .iter()
            .map(|(after, _)| *after)
            .filter(|after| *after > 0)
            .collect::<Vec<_>>();
        assert_eq!(scripted.len(), waits.len(), "{interaction:?}");
        // An operation lands once the run decodes `after` words, and the
        // run decodes the word at index `wait` as word `wait + 1`.
        for (after, wait) in scripted.iter().zip(waits) {
            assert!(
                after > wait,
                "{interaction:?}: lands before word {wait} parks"
            );
        }
    }
}

/// [SPU-ISA p:200 s:9.3] RN0 and RN1 each name one of four modes.
#[test]
fn random_states_draw_every_rounding_mode_in_both_slices() {
    use cellgov_ps3_abi::hw::spu_fpscr::FPSCR_DEFINED;
    let mut seen = [[false; 4]; 2];
    for case in 0..64 {
        let mut rng = Rng::for_case(crate::CAMPAIGN_VERSION, 11, case);
        let state = random_state(&mut rng).unwrap();
        assert_eq!(
            state.fpscr() & !FPSCR_DEFINED,
            0,
            "only defined bits are drawn"
        );
        for (slice, mode) in state.fpscr_rounding().into_iter().enumerate() {
            seen[slice][mode as usize] = true;
        }
    }
    assert_eq!(seen, [[true; 4]; 2]);
}
