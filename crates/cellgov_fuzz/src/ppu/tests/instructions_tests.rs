use super::*;
use cellgov_ppu::instruction::fuzz::PpuFuzzKind;
use cellgov_ppu::instruction::PpuInstructionKind;

use crate::campaign::{CampaignSchedule, CaseRange};
use crate::{ConfigurationError, RunOutcome};

fn schedule(count: u64) -> CampaignSchedule {
    CampaignSchedule {
        cases: CaseRange { first: 0, count },
        ..CampaignSchedule::default()
    }
}

#[test]
fn a_substituted_word_replaces_the_generated_word_of_every_case() {
    let ori_r3_r3_one = (24 << 26) | (3 << 21) | (3 << 16) | 1;
    let ori_only = BTreeSet::from([InstructionIdentity::Ppu(PpuFuzzKind::Ordinary(
        PpuInstructionKind::Ori,
    ))]);
    let config = FuzzConfig {
        schedule: schedule(3),
        ..FuzzConfig::default()
    };
    // The premise: the generator's own draw for these cases is not ori alone,
    // so an ignored substitution cannot pass the assertion below.
    let control = run_instructions(config);
    assert_eq!(control.report.cases, 3);
    assert_ne!(control.report.instruction_kinds, ori_only);

    let run = run_instructions_with(config, Some(&[ori_r3_r3_one]));

    assert_eq!(run.report.cases, 3);
    assert_eq!(run.report.instruction_kinds, ori_only);
}

#[test]
fn raw_case_words_are_the_generator_draw_and_structured_case_words_decode() {
    let raw = FuzzConfig {
        strategy: GenerationStrategy::RawWords,
        ..FuzzConfig::default()
    };
    for case in [0u64, 1, 4_095] {
        let mut rng = Rng::for_case(raw.campaign_version, raw.seed, case);
        assert_eq!(
            instruction_case_words(raw, case).expect("raw case words"),
            vec![rng.next_u32()],
            "case {case}"
        );
    }

    let structured = FuzzConfig {
        strategy: GenerationStrategy::Structured,
        ..FuzzConfig::default()
    };
    for case in 0..64u64 {
        let words = instruction_case_words(structured, case).expect("structured case words");
        assert_eq!(words.len(), 1, "case {case}");
        assert!(
            cellgov_ppu::decode::decode(words[0]).is_ok(),
            "case {case} word {:#010x}",
            words[0]
        );
    }
}

#[test]
fn a_zero_case_instruction_campaign_is_a_typed_refusal() {
    let run = run_instructions(FuzzConfig {
        schedule: schedule(0),
        ..FuzzConfig::default()
    });

    assert!(matches!(
        run.outcome,
        RunOutcome::HarnessFailure(FuzzError::Configuration(ConfigurationError::ZeroIterations))
    ));
    assert_eq!(run.report.cases, 0);
}

#[test]
fn relation_checks_have_stable_fingerprint_identities() {
    assert_eq!(
        relation_check(PpuMetamorphicRelation::RecordCr0),
        CheckIdentity::PpuRecordCr0
    );
    assert_eq!(
        relation_check(PpuMetamorphicRelation::RecordCr1),
        CheckIdentity::PpuRecordCr1
    );
    assert_eq!(
        relation_check(PpuMetamorphicRelation::RecordCr6),
        CheckIdentity::PpuRecordCr6
    );
    assert_eq!(
        relation_check(PpuMetamorphicRelation::OverflowEnable),
        CheckIdentity::PpuOverflowEnable
    );
}

#[test]
fn seeded_effect_and_outcome_leaks_receive_typed_divergence_classes() {
    let effect = [PpuObservationComponent::CommittedEffects]
        .into_iter()
        .collect();
    assert_eq!(
        metamorphic_divergence(&effect),
        (DivergenceClass::Effect, CrossReferenceAsymmetry::Effect)
    );

    let outcome = [PpuObservationComponent::Outcome].into_iter().collect();
    assert_eq!(
        metamorphic_divergence(&outcome),
        (DivergenceClass::Outcome, CrossReferenceAsymmetry::Outcome)
    );

    let mixed = [
        PpuObservationComponent::Outcome,
        PpuObservationComponent::CommittedEffects,
    ]
    .into_iter()
    .collect();
    assert_eq!(
        metamorphic_divergence(&mixed),
        (DivergenceClass::Effect, CrossReferenceAsymmetry::Effect)
    );

    let fault = [PpuObservationComponent::FaultDiscard]
        .into_iter()
        .collect();
    assert_eq!(
        metamorphic_divergence(&fault),
        (DivergenceClass::Outcome, CrossReferenceAsymmetry::Fault)
    );
}

#[test]
fn declared_relations_are_counted_as_executed_or_inapplicable() {
    for relation in [
        PpuMetamorphicRelation::RecordCr0,
        PpuMetamorphicRelation::RecordCr1,
        PpuMetamorphicRelation::RecordCr6,
        PpuMetamorphicRelation::OverflowEnable,
    ] {
        let generation = generation_descriptors()
            .into_iter()
            .find(|descriptor| {
                cellgov_ppu::decode::decode(descriptor.canonical_word).is_ok_and(|instruction| {
                    instruction
                        .fuzz_descriptor(descriptor.canonical_word)
                        .relations
                        .contains(&relation)
                })
            })
            .unwrap();
        let raw = generation.canonical_word
            & match relation {
                PpuMetamorphicRelation::RecordCr6 | PpuMetamorphicRelation::OverflowEnable => {
                    !0x0000_0401
                }
                _ => !0x0000_0001,
            };
        let instruction = cellgov_ppu::decode::decode(raw).unwrap();
        let descriptor = instruction.fuzz_descriptor(raw);
        let initial = PpuState::new();
        let memory = vec![0; DATA_LEN];
        let baseline = run_once(&instruction, &initial, &memory).unwrap();
        let config = FuzzConfig::default();
        let mut report = FuzzReport::new(
            FuzzTarget::PpuInstruction,
            config.seed,
            config.strategy,
            config.retention,
            config.max_findings as usize,
            config.sequence_words,
        );

        let asymmetry = run_metamorphic_checks(
            &mut report,
            MetamorphicRun {
                instruction: &instruction,
                descriptor,
                initial: &initial,
                memory: &memory,
                raw,
                identity: InstructionIdentity::Ppu(descriptor.kind),
                iteration: 0,
                baseline: &baseline,
            },
        )
        .unwrap();

        assert_eq!(asymmetry, CrossReferenceAsymmetry::None, "{relation:?}");
        let count = report
            .metamorphic_executions
            .get(&relation_check(relation))
            .copied()
            .unwrap_or(0)
            + report
                .metamorphic_inapplicable
                .get(&relation_check(relation))
                .copied()
                .unwrap_or(0);
        assert_eq!(count, 1, "{relation:?}");
        assert!(!report
            .findings
            .iter()
            .any(|finding| finding.kind == FindingKind::MetamorphicViolation));
    }
}

#[test]
fn undeclared_descriptor_relation_is_a_visible_finding() {
    let generation = generation_descriptors()
        .into_iter()
        .find(|descriptor| {
            descriptor.kind
                == cellgov_ppu::instruction::fuzz::PpuFuzzKind::Ordinary(
                    cellgov_ppu::instruction::PpuInstructionKind::Ori,
                )
        })
        .unwrap();
    let raw = generation.canonical_word;
    let instruction = cellgov_ppu::decode::decode(raw).unwrap();
    let mut descriptor = instruction.fuzz_descriptor(raw);
    descriptor.relations = &[PpuMetamorphicRelation::RecordCr0];
    let initial = PpuState::new();
    let memory = vec![0; DATA_LEN];
    let baseline = run_once(&instruction, &initial, &memory).unwrap();
    let config = FuzzConfig::default();
    let mut report = FuzzReport::new(
        FuzzTarget::PpuInstruction,
        config.seed,
        config.strategy,
        config.retention,
        config.max_findings as usize,
        config.sequence_words,
    );

    let asymmetry = run_metamorphic_checks(
        &mut report,
        MetamorphicRun {
            instruction: &instruction,
            descriptor,
            initial: &initial,
            memory: &memory,
            raw,
            identity: InstructionIdentity::Ppu(descriptor.kind),
            iteration: 0,
            baseline: &baseline,
        },
    )
    .unwrap();

    assert_eq!(asymmetry, CrossReferenceAsymmetry::Outcome);
    assert!(report.metamorphic_executions.is_empty());
    assert_eq!(
        report
            .finding_counts
            .get(&FindingKind::MetamorphicViolation),
        Some(&1)
    );
    assert_eq!(
        report.findings[0].fingerprint.check,
        CheckIdentity::PpuRecordCr0
    );
    assert_eq!(
        report.findings[0].fingerprint.divergence,
        DivergenceClass::ReferenceDisagreement
    );
    assert_eq!(report.findings[0].fingerprint.outcome, None);
}

#[test]
fn enabled_controls_are_counted_as_inapplicable_not_executed() {
    let generation = generation_descriptors()
        .into_iter()
        .find(|descriptor| {
            descriptor.kind
                == cellgov_ppu::instruction::fuzz::PpuFuzzKind::Ordinary(
                    cellgov_ppu::instruction::PpuInstructionKind::Add,
                )
        })
        .unwrap();
    let raw = generation.canonical_word | 1;
    let instruction = cellgov_ppu::decode::decode(raw).unwrap();
    let descriptor = instruction.fuzz_descriptor(raw);
    let initial = PpuState::new();
    let memory = vec![0; DATA_LEN];
    let baseline = run_once(&instruction, &initial, &memory).unwrap();
    let config = FuzzConfig::default();
    let mut report = FuzzReport::new(
        FuzzTarget::PpuInstruction,
        config.seed,
        config.strategy,
        config.retention,
        config.max_findings as usize,
        config.sequence_words,
    );

    let asymmetry = run_metamorphic_checks(
        &mut report,
        MetamorphicRun {
            instruction: &instruction,
            descriptor,
            initial: &initial,
            memory: &memory,
            raw,
            identity: InstructionIdentity::Ppu(descriptor.kind),
            iteration: 0,
            baseline: &baseline,
        },
    )
    .unwrap();

    assert_eq!(asymmetry, CrossReferenceAsymmetry::None);
    assert!(report.metamorphic_executions.is_empty());
    assert!(report.findings.is_empty());
    assert_eq!(
        report
            .metamorphic_inapplicable
            .get(&CheckIdentity::PpuRecordCr0),
        Some(&1)
    );
    assert_eq!(
        report
            .metamorphic_inapplicable
            .get(&CheckIdentity::PpuOverflowEnable),
        Some(&1)
    );
}
