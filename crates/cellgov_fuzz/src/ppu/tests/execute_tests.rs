use std::collections::BTreeSet;

use super::*;
use cellgov_ppu::instruction::fuzz::PpuFuzzKind;
use cellgov_ppu::instruction::PpuInstructionKind;

use crate::case::{CaseEligibility, EligibilityReason};
use crate::ppu::generate::{DATA_BASE, DATA_LEN};

fn mftb(rt: u32) -> u32 {
    (31 << 26) | (rt << 21) | (12 << 16) | (8 << 11) | (371 << 1)
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
    stores
        .insert(DATA_REGION_BASE - 4, 4, 0x1122_3344)
        .expect("staged");

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
fn replay_requires_the_deterministic_relation() {
    assert!(!requests_replay(&[]));
    assert!(requests_replay(&[PpuMetamorphicRelation::Deterministic]));
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
