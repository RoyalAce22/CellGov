use super::*;
use crate::{ConfigurationError, RetentionConfig, RunOutcome};
use cellgov_ppu::instruction::fuzz::{PpuFuzzKind, PpuSequenceDependency};
use cellgov_ppu::instruction::PpuInstructionKind;

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
    )
    .unwrap();

    assert_eq!(observed.observation.state.gpr[0], u64::from(initial.vrsave));
}

#[test]
fn a_time_base_read_is_part_of_the_observed_effects() {
    let initial = PpuState::new();
    let instruction = cellgov_ppu::decode::decode(mftb(3)).unwrap();
    let observed = run_once(&instruction, &initial, &[0; DATA_LEN]).unwrap();

    assert!(observed
        .observation
        .committed_effects
        .iter()
        .any(|effect| matches!(effect, Effect::ClockRead { source } if *source == UNIT)));
}

#[test]
fn a_ppu_sequence_retains_terminal_effects() {
    let initial = PpuState::new();
    let run = run_sequence(&[mftb(3)], &initial, &[0; DATA_LEN]).unwrap();

    assert_eq!(run.decoded, 1);
    assert_eq!(run.executed, 1);
    assert!(matches!(
        run.observed.terminal_verdict,
        Some(ExecuteVerdict::Continue)
    ));
    assert!(run
        .observed
        .observation
        .committed_effects
        .iter()
        .any(|effect| matches!(effect, Effect::ClockRead { source } if *source == UNIT)));
}

#[test]
fn a_commit_refusal_is_not_a_successful_fuzz_observation() {
    let state = PpuState::new();
    let mut stores = StoreBuffer::new();
    assert!(stores.insert(DATA_REGION_BASE - 4, 4, 0x1122_3344));

    let observation = finish_fuzz_observation(PpuObservationInput {
        initial_state: &state,
        final_state: &state,
        memory_base: DATA_REGION_BASE,
        initial_memory: &[0; DATA_LEN],
        outcome: PpuObservedOutcome::Execution(ExecuteVerdict::Continue),
        effects: Vec::new(),
        stores,
        unit: UNIT,
    })
    .unwrap();

    assert!(matches!(
        observation.commit_error,
        Some(PpuObservationError::WriteOutOfRange {
            kind: cellgov_effects::EffectKind::SharedWriteIntent,
            addr,
            len: 4,
        }) if addr == DATA_REGION_BASE - 4
    ));
    assert!(observation.committed_effects.is_empty());
    assert!(matches!(
        observation.staged_effects.as_slice(),
        [Effect::SharedWriteIntent { .. }]
    ));
    let observed = ObservedStep {
        verdict: ExecuteVerdict::Continue,
        observation,
    };
    let assessment = CaseAssessment::new(
        CaseEligibility::Eligible,
        EligibilityReason::InterpreterContract,
        BTreeSet::new(),
    );
    let semantic = ppu_observation(
        [],
        &assessment,
        PpuTerminalObservation::from_step(&observed),
        &observed.observation.committed_effects,
        1,
        ppu_step_changed(&state, &observed),
        CrossReferenceAsymmetry::None,
    );
    assert_eq!(semantic.outcome, Some(OutcomeIdentity::PpuCommitRefusal));
    assert_eq!(
        semantic.state_transition,
        StateTransitionClass::CommitRefused
    );
}

#[test]
fn a_ppu_sequence_fetches_from_the_branch_target() {
    let branch_over_next_word = (18 << 26) | 8;
    let li_r3_one = (14 << 26) | (3 << 21) | 1;
    let nop = 24 << 26;
    let initial = PpuState::new();

    let run = run_sequence(
        &[branch_over_next_word, li_r3_one, nop],
        &initial,
        &[0; DATA_LEN],
    )
    .unwrap();

    assert_eq!(run.decoded, 2);
    assert_eq!(run.executed, 2);
    assert_eq!(run.observed.observation.state.gpr[3], 0);
}

#[test]
fn a_faulting_ppu_sequence_discards_prior_state() {
    let li_r3_one = (14 << 26) | (3 << 21) | 1;
    let lwz_r4_r5 = (32 << 26) | (4 << 21) | (5 << 16);
    let initial = PpuState::new();

    let run = run_sequence(&[li_r3_one, lwz_r4_r5], &initial, &[0; DATA_LEN]).unwrap();

    assert_eq!(run.decoded, 2);
    assert_eq!(run.executed, 2);
    assert_eq!(run.observed.observation.state.gpr[3], 0);
    assert_eq!(run.observed.observation.state.pc, 0);
    assert!(run.observed.observation.committed_effects.is_empty());
    assert!(run.observed.observation.fault_discarded);
    assert!(matches!(
        run.observed.terminal_verdict,
        Some(ExecuteVerdict::MemFault(_))
    ));
}

#[test]
fn fault_discarded_effects_are_not_guest_visible() {
    let lwz_r3_r4 = (32 << 26) | (3 << 21) | (4 << 16);
    let lwz_r5_r6 = (32 << 26) | (5 << 21) | (6 << 16);
    let mut initial = PpuState::new();
    initial.set_gpr(4, DATA_BASE);

    let run = run_sequence(&[lwz_r3_r4, lwz_r5_r6], &initial, &[0; DATA_LEN]).unwrap();

    assert!(!run.observed.observation.staged_effects.is_empty());
    assert!(run.observed.observation.committed_effects.is_empty());
    let assessment = CaseAssessment::new(
        CaseEligibility::Eligible,
        EligibilityReason::InterpreterContract,
        BTreeSet::new(),
    );
    let observation = ppu_observation(
        run.executed_kinds.iter().copied(),
        &assessment,
        PpuTerminalObservation::from_sequence(&run.observed),
        &run.observed.observation.committed_effects,
        run.executed,
        ppu_sequence_changed(&initial, &run.observed),
        CrossReferenceAsymmetry::None,
    );

    assert_eq!(
        observation.state_transition,
        StateTransitionClass::FaultDiscarded
    );
    assert!(observation.effects.is_empty());
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

fn observed_step(verdict: ExecuteVerdict) -> ObservedStep {
    let state = PpuState::new();
    let observation = finish_observation(PpuObservationInput {
        initial_state: &state,
        final_state: &state,
        memory_base: DATA_REGION_BASE,
        initial_memory: &[0; DATA_LEN],
        outcome: PpuObservedOutcome::Execution(verdict.clone()),
        effects: Vec::new(),
        stores: StoreBuffer::new(),
        unit: UNIT,
    })
    .unwrap();
    ObservedStep {
        verdict,
        observation,
    }
}

#[test]
fn ppu_replay_asymmetry_uses_the_strongest_changed_axis() {
    let first = observed_step(ExecuteVerdict::Continue);

    assert_eq!(
        ppu_outcome_asymmetry(PpuOutcomeClass::Continue),
        CrossReferenceAsymmetry::Outcome
    );
    assert_eq!(
        ppu_outcome_asymmetry(PpuOutcomeClass::MemoryFault),
        CrossReferenceAsymmetry::Fault
    );

    let mut second = first.clone();
    second.observation.state.gpr[0] = 1;
    assert_eq!(
        ppu_step_replay_asymmetry(&first, &second),
        CrossReferenceAsymmetry::State
    );

    let mut second = first.clone();
    second.observation.outcome = PpuObservedOutcome::Execution(ExecuteVerdict::Branch);
    assert_eq!(
        ppu_step_replay_asymmetry(&first, &second),
        CrossReferenceAsymmetry::Outcome
    );

    let mut second = first.clone();
    second.observation.outcome = PpuObservedOutcome::Execution(ExecuteVerdict::Fault(
        cellgov_ppu::exec::PpuFault::UnimplementedInstruction(0),
    ));
    assert_eq!(
        ppu_step_replay_asymmetry(&first, &second),
        CrossReferenceAsymmetry::Fault
    );

    let mut second = first.clone();
    second.verdict = ExecuteVerdict::Branch;
    assert_eq!(
        ppu_step_replay_asymmetry(&first, &second),
        CrossReferenceAsymmetry::Outcome
    );

    let mut second = first.clone();
    second
        .observation
        .committed_effects
        .push(Effect::ClockRead { source: UNIT });
    assert_eq!(
        ppu_step_replay_asymmetry(&first, &second),
        CrossReferenceAsymmetry::Effect
    );

    let mut second = first.clone();
    second.verdict =
        ExecuteVerdict::Fault(cellgov_ppu::exec::PpuFault::UnimplementedInstruction(0));
    second
        .observation
        .committed_effects
        .push(Effect::ClockRead { source: UNIT });
    assert_eq!(
        ppu_step_replay_asymmetry(&first, &second),
        CrossReferenceAsymmetry::Fault
    );
}

#[test]
fn ppu_sequence_replay_compares_decoded_and_executed_depth_and_order() {
    let state = PpuState::new();
    let observed = ObservedSequence {
        observation: finish_observation(PpuObservationInput {
            initial_state: &state,
            final_state: &state,
            memory_base: DATA_REGION_BASE,
            initial_memory: &[0; DATA_LEN],
            outcome: PpuObservedOutcome::Execution(ExecuteVerdict::Continue),
            effects: Vec::new(),
            stores: StoreBuffer::new(),
            unit: UNIT,
        })
        .unwrap(),
        terminal_verdict: Some(ExecuteVerdict::Continue),
        decode_refusal: None,
        deterministic: true,
    };
    let ori = InstructionIdentity::Ppu(PpuFuzzKind::Ordinary(PpuInstructionKind::Ori));
    let xori = InstructionIdentity::Ppu(PpuFuzzKind::Ordinary(PpuInstructionKind::Xori));
    let first = ObservedSequenceRun {
        observed,
        decoded: 2,
        decoded_kinds: vec![ori, xori],
        executed: 2,
        executed_kinds: vec![ori, xori],
    };
    let mut decoded_reordered = first.clone();
    decoded_reordered.decoded_kinds.reverse();
    let mut executed_reordered = first.clone();
    executed_reordered.executed_kinds.reverse();
    let mut decoded_deeper = first.clone();
    decoded_deeper.decoded = 3;
    let mut executed_deeper = first.clone();
    executed_deeper.executed = 3;
    let mut refusal = first.clone();
    refusal.observed.decode_refusal = Some((8, u32::MAX));
    let mut outcome = first.clone();
    outcome.observed.observation.outcome = PpuObservedOutcome::NoInstruction;

    assert_eq!(
        ppu_sequence_replay_asymmetry(&first, &decoded_reordered),
        CrossReferenceAsymmetry::State
    );
    assert_eq!(
        ppu_sequence_replay_asymmetry(&first, &executed_reordered),
        CrossReferenceAsymmetry::State
    );
    assert_eq!(
        ppu_sequence_replay_asymmetry(&first, &decoded_deeper),
        CrossReferenceAsymmetry::State
    );
    assert_eq!(
        ppu_sequence_replay_asymmetry(&first, &executed_deeper),
        CrossReferenceAsymmetry::State
    );
    assert_eq!(
        ppu_sequence_replay_asymmetry(&first, &refusal),
        CrossReferenceAsymmetry::Outcome
    );
    assert_eq!(
        ppu_sequence_replay_asymmetry(&first, &outcome),
        CrossReferenceAsymmetry::Outcome
    );
}

#[test]
fn buffer_full_is_decoded_but_not_counted_as_executed() {
    let stw_r3_at_r4 = (36 << 26) | (3 << 21) | (4 << 16);
    let sth_r3_at_r4 = (44 << 26) | (3 << 21) | (4 << 16);
    let mut words = vec![stw_r3_at_r4; 64];
    words.push(sth_r3_at_r4);
    let mut initial = PpuState::new();
    initial.set_gpr(4, DATA_BASE);

    let run = run_sequence(&words, &initial, &[0; DATA_LEN]).unwrap();

    assert_eq!(run.decoded, 65);
    assert_eq!(run.decoded_kinds.len(), 65);
    assert_eq!(run.executed, 64);
    assert_eq!(run.executed_kinds.len(), 64);
    assert_ne!(run.decoded_kinds.last(), run.executed_kinds.last());
    assert!(matches!(
        run.observed.terminal_verdict.as_ref(),
        Some(&ExecuteVerdict::BufferFull)
    ));

    let assessment = CaseAssessment::new(
        CaseEligibility::Eligible,
        EligibilityReason::InterpreterContract,
        BTreeSet::new(),
    );
    let observation = ppu_observation(
        run.executed_kinds.iter().copied(),
        &assessment,
        PpuTerminalObservation::from_sequence(&run.observed),
        &run.observed.observation.committed_effects,
        run.executed,
        ppu_sequence_changed(&initial, &run.observed),
        CrossReferenceAsymmetry::None,
    );
    assert_eq!(observation.sequence_depth, 64);
    assert_eq!(observation.instruction_kinds.len(), 1);
}

#[test]
fn ppu_observation_preserves_the_first_executed_kind() {
    let ori = InstructionIdentity::Ppu(PpuFuzzKind::Ordinary(PpuInstructionKind::Ori));
    let xori = InstructionIdentity::Ppu(PpuFuzzKind::Ordinary(PpuInstructionKind::Xori));
    let assessment = CaseAssessment::new(
        CaseEligibility::Eligible,
        EligibilityReason::InterpreterContract,
        BTreeSet::new(),
    );

    let observation = ppu_observation(
        [xori, ori],
        &assessment,
        PpuTerminalObservation::Execution(Some(&ExecuteVerdict::Continue)),
        &[],
        2,
        false,
        CrossReferenceAsymmetry::None,
    );

    assert_eq!(observation.first_instruction_kind, Some(xori));

    let refusal = ppu_observation(
        [xori, ori],
        &assessment,
        PpuTerminalObservation::DecodeRefusal(Some(&ExecuteVerdict::Continue)),
        &[],
        2,
        false,
        CrossReferenceAsymmetry::None,
    );
    assert_eq!(refusal.outcome, Some(OutcomeIdentity::PpuDecodeRefusal));
    assert_eq!(
        refusal.state_transition,
        StateTransitionClass::FaultDiscarded
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

fn generation_descriptor_for(
    descriptors: &[PpuGenerationDescriptor],
    kind: PpuInstructionKind,
) -> PpuGenerationDescriptor {
    descriptors
        .iter()
        .find(|descriptor| descriptor.kind == PpuFuzzKind::Ordinary(kind))
        .cloned()
        .expect("generation descriptor must exist")
}

#[test]
fn structured_generation_retries_invalid_ppu_register_relations() {
    let mut rng = Rng::for_case(crate::CAMPAIGN_VERSION, 0x5eed, 0);
    let descriptors = generation_descriptors();

    let load_update = generation_descriptor_for(&descriptors, PpuInstructionKind::Lwzu);
    for _ in 0..512 {
        let raw = structured_word(std::slice::from_ref(&load_update), &mut rng).unwrap();
        let PpuInstruction::Lwzu { rt, ra, .. } = cellgov_ppu::decode::decode(raw).unwrap() else {
            panic!("selected descriptor must preserve lwzu");
        };
        assert_ne!(ra, 0);
        assert_ne!(ra, rt);
    }

    let store_update = generation_descriptor_for(&descriptors, PpuInstructionKind::Stwu);
    for _ in 0..512 {
        let raw = structured_word(std::slice::from_ref(&store_update), &mut rng).unwrap();
        let PpuInstruction::Stwu { ra, .. } = cellgov_ppu::decode::decode(raw).unwrap() else {
            panic!("selected descriptor must preserve stwu");
        };
        assert_ne!(ra, 0);
    }

    let load_multiple = generation_descriptor_for(&descriptors, PpuInstructionKind::Lmw);
    for _ in 0..512 {
        let raw = structured_word(std::slice::from_ref(&load_multiple), &mut rng).unwrap();
        let PpuInstruction::Lmw { rt, ra, .. } = cellgov_ppu::decode::decode(raw).unwrap() else {
            panic!("selected descriptor must preserve lmw");
        };
        assert_ne!(ra, 0);
        assert!(ra < rt);
    }

    let string_load = generation_descriptor_for(&descriptors, PpuInstructionKind::Lswx);
    for _ in 0..512 {
        let raw = structured_word(std::slice::from_ref(&string_load), &mut rng).unwrap();
        let instruction = cellgov_ppu::decode::decode(raw).unwrap();
        let PpuInstruction::Lswx { rt, ra, rb } = instruction else {
            panic!("selected descriptor must preserve lswx");
        };
        let state = random_state_for_instruction(&instruction, &mut rng).unwrap();
        assert_ne!(state.xer_tbc(), 0);
        assert!(lswx_registers_are_valid(rt, ra, rb, state.xer_tbc()));
    }
}

#[test]
fn structured_generation_reports_constraint_retry_exhaustion() {
    let descriptors = generation_descriptors();
    let mut impossible = generation_descriptor_for(&descriptors, PpuInstructionKind::Lwzu);
    impossible.kind = PpuFuzzKind::Ordinary(PpuInstructionKind::Stwu);
    let mut rng = Rng::for_case(crate::CAMPAIGN_VERSION, 0x5eed, 1);

    assert_eq!(
        structured_word(std::slice::from_ref(&impossible), &mut rng),
        Err(GeneratorError::ConstraintAttemptsExhausted {
            target: "PPU",
            attempts: STRUCTURED_ENCODING_ATTEMPTS,
        })
    );
}

#[test]
fn structured_sequence_prefixes_exclude_state_dependent_xer_reads() {
    let descriptors = generation_descriptors();
    let mut rng = Rng::for_case(crate::CAMPAIGN_VERSION, 0x5eed, 2);

    for _ in 0..128 {
        let words = structured_words(&descriptors, &mut rng, 64).unwrap();
        let kinds = words
            .iter()
            .map(|raw| PpuInstructionKind::from(cellgov_ppu::decode::decode(*raw).unwrap()))
            .collect::<Vec<_>>();
        assert!(!kinds.contains(&PpuInstructionKind::Lswx));
        for (index, raw) in words.iter().enumerate() {
            let flow = cellgov_ppu::instruction::fuzz::generation_descriptor(*raw)
                .expect("structured word must retain a descriptor")
                .sequence_flow;
            if index + 1 < words.len() {
                assert_eq!(flow, PpuSequenceFlow::Linear);
            } else {
                assert!(matches!(
                    flow,
                    PpuSequenceFlow::Linear | PpuSequenceFlow::ControlTransfer
                ));
            }
        }
        assert_eq!(
            random_state_for_sequence(GenerationStrategy::Structured, &mut rng)
                .unwrap()
                .xer_tbc(),
            1
        );
    }
}

#[test]
fn dependency_chain_feature_requires_read_write_gpr_forms() {
    let descriptors = generation_descriptors();
    let mut rng = Rng::for_case(crate::CAMPAIGN_VERSION, 0x5eed, 3);
    let mut saw_chain = false;

    for _ in 0..128 {
        let generated = structured_sequence(&descriptors, &mut rng, 8).unwrap();
        if !generated.features.contains(&CaseFeature::DependencyChain) {
            continue;
        }
        saw_chain = true;
        let mut chain_register = None;
        for raw in generated.words {
            let descriptor = cellgov_ppu::instruction::fuzz::generation_descriptor(raw)
                .expect("structured word must retain a descriptor");
            assert_eq!(
                descriptor.sequence_dependency,
                Some(PpuSequenceDependency::GeneralPurposeRegister)
            );
            let (destination, source) = match cellgov_ppu::decode::decode(raw).unwrap() {
                PpuInstruction::Ori { ra, rs, .. }
                | PpuInstruction::Oris { ra, rs, .. }
                | PpuInstruction::Xori { ra, rs, .. }
                | PpuInstruction::Xoris { ra, rs, .. } => (ra, rs),
                instruction => panic!("dependency descriptor produced {instruction:?}"),
            };
            assert_eq!(destination, source);
            assert_eq!(*chain_register.get_or_insert(source), source);
        }
    }
    assert!(saw_chain);
}

#[test]
fn equal_cross_bank_field_values_are_not_reported_as_operand_aliases() {
    let descriptors = generation_descriptors();
    let logical = generation_descriptor_for(&descriptors, PpuInstructionKind::Ori);
    let float_load = generation_descriptor_for(&descriptors, PpuInstructionKind::Lfs);
    let mut rng = Rng::for_case(crate::CAMPAIGN_VERSION, 0x5eed, 4);

    let logical_parameters = generated_ppu_parameters(&logical, &mut rng, Some(7)).unwrap();
    assert!(logical_parameters
        .features
        .contains(&CaseFeature::OperandAlias));
    let float_parameters = generated_ppu_parameters(&float_load, &mut rng, Some(7)).unwrap();
    assert!(!float_parameters
        .features
        .contains(&CaseFeature::OperandAlias));
}

#[test]
fn architecturally_undefined_ppu_state_is_classified_before_replay() {
    let mut state = PpuState::new();
    state.set_gpr(4, 7);
    state.set_gpr(5, 0);
    let instruction = PpuInstruction::Divd {
        rt: 3,
        ra: 4,
        rb: 5,
        oe: false,
        rc: false,
    };
    let descriptor = instruction.fuzz_descriptor(0);
    let assessment = assess_instruction_case(
        GenerationStrategy::Structured,
        &instruction,
        &state,
        descriptor,
        &ExecuteVerdict::Continue,
        BTreeSet::new(),
    );

    assert_eq!(assessment.eligibility, CaseEligibility::Undefined);
    assert!(assessment
        .reasons
        .contains(&EligibilityReason::ArchitecturallyUndefined));
    assert!(PpuInstruction::Divw {
        rt: 3,
        ra: 4,
        rb: 4,
        oe: false,
        rc: false,
    }
    .fuzz_case_is_architecturally_undefined(&state));
    assert!(PpuInstruction::Mfocrf { rt: 3, crm: 3 }.fuzz_case_is_architecturally_undefined(&state));
    state.set_gpr(5, 2);
    assert!(!instruction.fuzz_case_is_architecturally_undefined(&state));
}

#[test]
fn an_unexpected_fault_remains_eligible_for_contract_checks() {
    let instruction = PpuInstruction::Addi {
        rt: 3,
        ra: 4,
        imm: 1,
    };
    let descriptor = instruction.fuzz_descriptor(14 << 26);
    let assessment = assess_instruction_case(
        GenerationStrategy::Structured,
        &instruction,
        &PpuState::new(),
        descriptor,
        &ExecuteVerdict::Fault(cellgov_ppu::exec::PpuFault::UnimplementedInstruction(14)),
        BTreeSet::new(),
    );

    assert_eq!(assessment.eligibility, CaseEligibility::Eligible);
}

#[test]
fn raw_words_do_not_claim_an_intentional_fault_boundary() {
    let instruction = PpuInstruction::Popcntb { ra: 3, rs: 4 };
    let descriptor = instruction.fuzz_descriptor(31 << 26);
    let assessment = assess_instruction_case(
        GenerationStrategy::RawWords,
        &instruction,
        &PpuState::new(),
        descriptor,
        &ExecuteVerdict::Fault(cellgov_ppu::exec::PpuFault::UnimplementedInstruction(122)),
        BTreeSet::new(),
    );

    assert_eq!(assessment.eligibility, CaseEligibility::Eligible);
    assert!(!assessment
        .features
        .contains(&CaseFeature::NamedFaultBoundary));
    assert!(assessment
        .reasons
        .contains(&EligibilityReason::DecoderRobustness));
}

#[test]
fn a_memory_fault_retracts_unmet_state_features() {
    let instruction = PpuInstruction::Lwz {
        rt: 3,
        ra: 0,
        imm: 0,
    };
    let state = PpuState::new();
    let observed = run_once(&instruction, &state, &[0; DATA_LEN]).unwrap();
    assert!(matches!(observed.verdict, ExecuteVerdict::MemFault(_)));
    let assessment = assess_instruction_case(
        GenerationStrategy::Structured,
        &instruction,
        &state,
        instruction.fuzz_descriptor(32 << 26),
        &observed.verdict,
        [CaseFeature::MappedMemory, CaseFeature::Reservation]
            .into_iter()
            .collect(),
    );

    assert_eq!(assessment.eligibility, CaseEligibility::Unsupported);
    assert!(!assessment.features.contains(&CaseFeature::MappedMemory));
    assert!(!assessment.features.contains(&CaseFeature::Reservation));
}
