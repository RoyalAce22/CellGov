use super::*;

fn observed_step(outcome: SpuStepOutcome) -> ObservedStep {
    ObservedStep {
        outcome,
        state: SpuObservableSnapshot::capture(&SpuState::new()),
    }
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
