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
