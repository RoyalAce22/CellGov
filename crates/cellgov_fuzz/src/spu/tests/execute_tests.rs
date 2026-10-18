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
    initial.channels.in_mbox = vec![0x0000_0042];

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
    // rdch MFC_RdTagStat with no update request: a stall nothing can
    // end, refused by name.
    let refused_rdch = (0x00Du32 << 21) | (u32::from(spu::MFC_RD_TAG_STAT) << 7) | 2;
    initial.ls[..4].copy_from_slice(&il_r3_one.to_be_bytes());
    initial.ls[4..8].copy_from_slice(&refused_rdch.to_be_bytes());

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
fn a_faulting_spu_sequence_discards_the_stores_the_world_committed() {
    const BASE: u64 = 0x1_0000;
    let mut initial = SpuState::new();
    // wrch MFC_Cmd from r2, then the refused tag-status read.
    let wrch_cmd = (0x10Du32 << 21) | (u32::from(spu::MFC_CMD) << 7) | 2;
    let refused_rdch = (0x00Du32 << 21) | (u32::from(spu::MFC_RD_TAG_STAT) << 7) | 2;
    initial.ls[..4].copy_from_slice(&wrch_cmd.to_be_bytes());
    initial.ls[4..8].copy_from_slice(&refused_rdch.to_be_bytes());
    initial.ls[0x100..0x110].fill(0xAA);
    initial.set_reg_word_splat(2, spu::MFC_PUT);
    initial.channels.mfc_lsa = 0x100;
    initial.channels.mfc_eal = BASE as u32;
    initial.channels.mfc_size = 16;
    let world = SequenceWorld {
        memory: vec![(BASE, vec![0; 0x100])],
        actions: Vec::new(),
    };

    let put_only = run_sequence_with_limit(&initial, &world, 1, None, &Cell::new(None))
        .expect("the world builds");
    let faulted = run_sequence_with_limit(&initial, &world, 2, None, &Cell::new(None))
        .expect("the world builds");

    // The put alone lands in main storage.
    assert_eq!(put_only.0.memory[0].1[..16], [0xAA; 16]);
    assert!(!put_only.0.effects.is_empty());
    // A fault later in the batch discards it with everything else.
    assert!(matches!(
        faulted.0.terminal_outcome,
        Some(SpuStepOutcome::Fault(_))
    ));
    assert_eq!(faulted.0.memory, world.memory);
    assert!(faulted.0.effects.is_empty());
}

#[test]
fn self_modified_decode_refusal_is_a_terminal_observation() {
    let mut initial = SpuState::new();
    // RR opcode 0x004 names no instruction, so the stored word never decodes.
    initial.set_reg(
        0,
        [0x00, 0x80, 0x00, 0x00]
            .repeat(4)
            .try_into()
            .expect("16 bytes"),
    );
    let stqd_r0_at_16 = 0x2400_4080u32;
    let nop = 0x4020_007fu32;
    for (index, word) in [stqd_r0_at_16, nop, nop, nop, nop].iter().enumerate() {
        let start = index * 4;
        initial.ls[start..start + 4].copy_from_slice(&word.to_be_bytes());
    }

    let (observed, decoded, _) = run_sequence(&initial, 5);

    assert_eq!(decoded, 4);
    assert_eq!(observed.decode_refusal, Some((16, 0x0080_0000)));
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
        cellgov_spu::exec::SpuFault::UnsupportedChannelCount(7),
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
        memory: Vec::new(),
        effects: Vec::new(),
        ppu_results: Vec::new(),
        mfc_exceptions: Vec::new(),
        stalled: false,
        parks: 0,
        coverage: SpuFacilityCoverage::default(),
    };
    let nop = InstructionIdentity::Spu(cellgov_spu::instruction::SpuInstructionKind::Nop);
    let lnop = InstructionIdentity::Spu(cellgov_spu::instruction::SpuInstructionKind::Lnop);
    let first = (observed.clone(), 2, vec![nop, lnop]);
    let mut parked = first.clone();
    parked.0.stalled = true;
    let mut stored = first.clone();
    stored.0.memory = vec![(0, vec![1])];
    let mut covered = first.clone();
    covered.0.coverage.channels.insert(spu::SPU_RD_EVENT_STAT);
    assert_eq!(
        spu_sequence_replay_asymmetry(&first, &covered),
        CrossReferenceAsymmetry::State
    );
    assert_eq!(
        spu_sequence_replay_asymmetry(&first, &parked),
        CrossReferenceAsymmetry::State
    );
    assert_eq!(
        spu_sequence_replay_asymmetry(&first, &stored),
        CrossReferenceAsymmetry::State
    );
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
    let initial = SpuState::new();
    let state = SpuObservableSnapshot::capture(&initial);

    let observation = spu_observation(
        [lnop, nop],
        &assessment,
        SpuTerminalObservation::Execution(Some(&SpuStepOutcome::Continue)),
        &state,
        &initial,
        2,
        CrossReferenceAsymmetry::None,
    );

    assert_eq!(observation.first_instruction_kind, Some(lnop));

    let refusal = spu_observation(
        [lnop, nop],
        &assessment,
        SpuTerminalObservation::DecodeRefusal(Some(&SpuStepOutcome::Continue)),
        &state,
        &initial,
        2,
        CrossReferenceAsymmetry::None,
    );
    assert_eq!(refusal.outcome, Some(OutcomeIdentity::SpuDecodeRefusal));
}

/// The message the recipe world delivers to a drained inbound mailbox.
const RECIPE_MESSAGE: u32 = 0x0BAD_F00D;

/// Bytes in one lock line.
const LINE: usize = RESERVATION_LINE_BYTES as usize;

/// The main-storage line the recipe world holds at the data base.
fn recipe_line() -> Vec<u8> {
    (0..LINE as u8).collect()
}

/// Main storage at the data base, and the operations an interaction
/// waits on.
fn recipe_world(interaction: SpuSequenceInteraction) -> SequenceWorld {
    let mut bytes = vec![0; crate::spu::generate::SEQUENCE_MAIN_STORAGE_BYTES];
    bytes[..LINE].copy_from_slice(&recipe_line());
    let actions = match interaction {
        SpuSequenceInteraction::EventWait => vec![(
            2,
            SpuReferencePpuAction::Signal {
                register: 1,
                value: 7,
            },
        )],
        SpuSequenceInteraction::MailboxDrain => vec![
            (2, SpuReferencePpuAction::ReadOutMbox),
            (
                4,
                SpuReferencePpuAction::InMbox {
                    value: RECIPE_MESSAGE,
                },
            ),
        ],
        _ => Vec::new(),
    };
    SequenceWorld {
        memory: vec![(u64::from(STRUCTURED_LS_DATA_BASE), bytes)],
        actions,
    }
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
        initial.set_reservation(Some(ReservedLine::containing(u64::from(
            STRUCTURED_LS_DATA_BASE,
        ))));
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
        let world = recipe_world(interaction);
        let first = run_generated_sequence(
            &initial,
            &world,
            words.len(),
            words.len(),
            GenerationStrategy::Structured,
            &Cell::new(None),
        )
        .expect("the world builds");
        let replay = run_generated_sequence(
            &initial,
            &world,
            words.len(),
            words.len(),
            GenerationStrategy::Structured,
            &Cell::new(None),
        )
        .expect("the world builds");
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
                assert_eq!(
                    first.0.state.regs[3][..4],
                    cellgov_spu::fuzz::SEQUENCE_MAILBOX_MESSAGE.to_be_bytes()
                );
                assert!(first.0.state.channels.in_mbox.is_empty());
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
                    assert!(matches!(
                        &first.0.terminal_outcome,
                        Some(SpuStepOutcome::Yield { effects, .. }) if matches!(
                            effects.first(),
                            Some(cellgov_effects::Effect::DmaEnqueue { request, payload: None })
                                if request.main_storage_write().is_none()
                        )
                    ));
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
            SpuSequenceInteraction::RefusedMfcCommand => {
                assert_eq!(first.2.len(), 2);
                assert!(
                    matches!(
                        &first.0.terminal_outcome,
                        Some(SpuStepOutcome::Yield { effects, .. })
                            if matches!(
                                effects.as_slice(),
                                [cellgov_effects::Effect::MfcInvalidCommand { command, .. }]
                                    if command.word == cellgov_spu::fuzz::REFUSED_MFC_OPCODE
                                        && command.error.to_string() == "opcode 0x00d8 is illegal"
                            )
                    ),
                    "{:?}",
                    first.0.terminal_outcome
                );
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
            SpuSequenceInteraction::TagWait => {
                assert_eq!(first.2.len(), 5);
                let base = STRUCTURED_LS_DATA_BASE as usize;
                // The get landed the main-storage line in local store.
                assert_eq!(first.0.state.ls[base..base + LINE], recipe_line());
                // Tag group 0 reads complete once the get lands.
                assert_eq!(first.0.state.regs[3][..4], 1u32.to_be_bytes());
                assert!(!first.0.stalled);
                // Every get completes in the step that queued it, so the
                // wait never blocks.
                assert_eq!(first.0.parks, 0);
                assert!(first
                    .0
                    .coverage
                    .mfc_opcodes
                    .contains(&(spu::MFC_GET as u16)));
            }
            SpuSequenceInteraction::ReserveStore => {
                assert_eq!(first.2.len(), 5);
                assert_eq!(
                    first.0.state.regs[3][..4],
                    spu::MFC_ATOMIC_STAT_G.to_be_bytes()
                );
                // [CBEA p:131 s:9.4] a putllc that kept its reservation reads 0.
                assert_eq!(first.0.state.regs[5][..4], 0u32.to_be_bytes());
                assert!(first
                    .0
                    .effects
                    .iter()
                    .any(|effect| matches!(effect, Effect::ConditionalStore { .. })));
                assert!(first.0.state.reservation.is_none());
            }
            SpuSequenceInteraction::EventWait => {
                assert_eq!(first.2.len(), 3);
                // The wait parked until the scripted signal write landed.
                assert_eq!(
                    first.0.ppu_results,
                    [crate::spu_reference::SpuReferencePpuResult::Done]
                );
                assert_eq!(first.0.state.regs[3][..4], spu::event::S1.to_be_bytes());
                assert_eq!(first.0.state.facilities.events.pending & spu::event::S1, 0);
                assert!(first.0.coverage.event_sources.contains(&spu::event::S1));
                assert!(!first.0.stalled);
                assert_eq!(first.0.parks, 1);
            }
            SpuSequenceInteraction::MailboxDrain => {
                use cellgov_spu::fuzz::SEQUENCE_MAILBOX_MESSAGE as MESSAGE;
                assert_eq!(first.2.len(), 4);
                assert_eq!(
                    first.0.ppu_results,
                    [
                        crate::spu_reference::SpuReferencePpuResult::Read {
                            value: Some(MESSAGE ^ 1)
                        },
                        crate::spu_reference::SpuReferencePpuResult::Done,
                    ]
                );
                assert_eq!(first.0.state.channels.out_mbox, Some(MESSAGE ^ 2));
                assert_eq!(first.0.state.regs[3][..4], MESSAGE.to_be_bytes());
                // The second inbound read parked until the scripted message landed.
                assert_eq!(first.0.state.regs[6][..4], RECIPE_MESSAGE.to_be_bytes());
                assert!(!first.0.stalled);
                // The second outbound write and the second inbound read each
                // parked once.
                assert_eq!(first.0.parks, 2);
            }
            SpuSequenceInteraction::Stop => {
                assert_eq!(first.2.len(), 2);
                // STOP's outcome carries its signal, so the campaign models the case.
                assert!(!first.0.has_unmodeled_execution);
                assert_eq!(
                    assess_sequence_case(
                        GenerationStrategy::Structured,
                        first.1,
                        false,
                        first.0.has_unmodeled_execution,
                        BTreeSet::new()
                    )
                    .eligibility,
                    CaseEligibility::Eligible
                );
                // [SPU-ISA p:238 s:10. Control Instructions] the stop at 4 leaves PC at 8.
                assert_eq!(first.0.state.pc, 8);
                assert_eq!(
                    first.0.state.stop.map(|stop| (stop.kind, stop.npc)),
                    Some((cellgov_spu::stop::SpuStopKind::Stop, 8))
                );
                assert!(matches!(
                    first.0.terminal_outcome,
                    Some(SpuStepOutcome::Stop {
                        kind: cellgov_spu::stop::SpuStopKind::Stop,
                        ..
                    })
                ));
            }
        }
        let mut defective = replay;
        match interaction {
            SpuSequenceInteraction::Channel => defective.0.state.channels.mfc_lsa ^= 16,
            SpuSequenceInteraction::Mailbox => defective.0.state.regs[3] = initial.regs[3],
            SpuSequenceInteraction::Dma => {
                if let Some(SpuStepOutcome::Yield { effects, .. }) =
                    &mut defective.0.terminal_outcome
                {
                    effects.clear();
                }
            }
            SpuSequenceInteraction::DmaGet => {
                if let Some(SpuStepOutcome::Yield { effects, .. }) =
                    &mut defective.0.terminal_outcome
                {
                    effects.clear();
                }
            }
            SpuSequenceInteraction::MemoryRead => {
                defective.0.terminal_outcome = Some(SpuStepOutcome::Continue)
            }
            SpuSequenceInteraction::Reservation => {
                defective.0.state.reservation = initial.reservation()
            }
            SpuSequenceInteraction::LocalStore => {
                defective.0.state.ls[STRUCTURED_LS_DATA_BASE as usize] ^= 1
            }
            SpuSequenceInteraction::RefusedMfcCommand => defective.0.state.pc ^= 4,
            SpuSequenceInteraction::Branch => defective.0.state.pc ^= 4,
            SpuSequenceInteraction::Stop => {
                defective.0.terminal_outcome = Some(SpuStepOutcome::Continue)
            }
            SpuSequenceInteraction::TagWait | SpuSequenceInteraction::EventWait => {
                defective.0.state.regs[3][3] ^= 1
            }
            SpuSequenceInteraction::ReserveStore => defective.0.effects.clear(),
            SpuSequenceInteraction::MailboxDrain => defective.0.ppu_results.clear(),
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
        &SequenceWorld::default(),
        2,
        2,
        GenerationStrategy::Structured,
        &Cell::new(None),
    )
    .expect("an empty world builds");

    assert_eq!(decoded, 1);
}
