//! Checks the interpreter, internal relation, and independent SPU reference together.

use cellgov_event::UnitId;
use cellgov_fuzz::spu_reference::{
    compare_reference, parse_reference_json, replay_reference, SpuReferenceComponent,
};
use cellgov_fuzz::{spu, CampaignSchedule, CaseRange, FuzzConfig, RunOutcome};
use cellgov_spu::decode::decode;
use cellgov_spu::exec::{execute, SpuStepOutcome};
use cellgov_spu::fuzz::{SpuMetamorphicRelation, SpuOutcomeClass};
use cellgov_spu::observation::{SpuAllowedFootprint, SpuObservation};
use cellgov_spu::state::SpuState;

const VECTOR: &str = include_str!("fixtures/spu_reference/rotqbyi_12_v1.json");

#[test]
fn a_common_mode_spu_defect_crosses_internal_checks_but_not_a_documented_vector() {
    let artifact = parse_reference_json(VECTOR).expect("official-source vector must parse");
    let reference = replay_reference(&artifact).expect("vector must replay offline");
    assert!(reference.comparison.is_match());

    let word = artifact.words[0];
    let instruction = decode(word).expect("committed word must decode");
    let descriptor = instruction.fuzz_descriptor();
    assert!(descriptor.decoded_execution_supported);
    assert!(descriptor.outcomes.contains(&SpuOutcomeClass::Continue));
    assert!(descriptor
        .relations
        .contains(&SpuMetamorphicRelation::RotateByteCountHighBit));
    let partner_word = instruction
        .metamorphic_case(word, SpuMetamorphicRelation::RotateByteCountHighBit)
        .expect("documented upper immediate bits permit a partner")
        .partner_word;
    let partner = decode(partner_word).expect("partner must decode");

    let mut initial = SpuState::new();
    initial.regs = reference.initial.regs;
    initial.ls = reference.initial.ls.clone();
    initial.pc = reference.initial.pc;
    let mut original_state = initial.clone();
    let mut partner_state = initial.clone();
    let original_outcome = execute(&instruction, &mut original_state, UnitId::new(0));
    let partner_outcome = execute(&partner, &mut partner_state, UnitId::new(0));
    assert!(matches!(original_outcome, SpuStepOutcome::Continue));
    let original = SpuObservation::capture(&original_state, &original_outcome);
    let alternate = SpuObservation::capture(&partner_state, &partner_outcome);
    assert!(original.compare(&alternate).differences.is_empty());
    assert!(SpuAllowedFootprint::for_instruction(&instruction)
        .violations(&initial, &original)
        .is_empty());
    assert_eq!(reference.state.regs, original.state.regs);

    let mut defective_original = original;
    let mut defective_partner = alternate;
    defective_original.state.regs[4][0] ^= 1;
    defective_partner.state.regs[4][0] ^= 1;
    assert!(defective_original
        .compare(&defective_partner)
        .differences
        .is_empty());
    // The single-instruction executor leaves PC advancement to its caller.
    let next_pc = initial
        .pc
        .checked_add(4)
        .expect("fixture PC must have a next word");
    assert_eq!(reference.state.pc, next_pc);
    let mut normalized = defective_original.state.clone();
    normalized.pc = next_pc;
    let independent = compare_reference(
        &artifact.expected,
        &reference.initial,
        &normalized,
        &defective_original.outcome,
    )
    .expect("typed comparison must run");
    assert_eq!(
        independent.differences,
        [SpuReferenceComponent::Registers].into()
    );
}

#[test]
fn stateful_spu_campaign_replays_with_complete_observations() {
    let config = FuzzConfig {
        seed: 7,
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 0,
                count: 256,
            },
            ..CampaignSchedule::default()
        },
        sequence_words: 4,
        ..FuzzConfig::default()
    };
    let first = spu::run_sequences(config);
    assert_eq!(first, spu::run_sequences(config));
    assert_eq!(first.outcome, RunOutcome::CleanCompletion);
    assert_eq!(first.report.cases, 256);
    assert!(first.report.finding_counts.is_empty());
    assert!(first.report.executed_steps > first.report.cases);
}
