//! Tests dependency-rich PPU sequence generation and replay metadata.

use cellgov_fuzz::ppu_paths::first_path_divergence;
use cellgov_fuzz::ppu_sequences::{
    generate_dependency_sequence, replay_dependency_sequence, run_dependency_campaign,
    PpuSequenceFamily, PpuSequenceStopClass,
};

#[test]
fn every_sequence_family_executes_and_detects_a_seeded_path_defect() {
    for (case_index, family) in PpuSequenceFamily::ALL.into_iter().enumerate() {
        let generated = generate_dependency_sequence(0x1137, case_index as u64);
        assert_eq!(generated.family, family);
        assert_eq!(generated.reduction_boundaries.len(), 1);
        assert_eq!(generated.reduction_boundaries[0].start, 0);
        assert_eq!(generated.reduction_boundaries[0].end, generated.words.len());
        assert_eq!(
            generated.assessment.eligibility,
            cellgov_fuzz::CaseEligibility::Eligible
        );

        let replay = replay_dependency_sequence(generated).expect("family must replay");
        assert!(replay.runs[0].retired > 0, "family {family:?}");
        match family {
            PpuSequenceFamily::RegisterAlias => {
                assert_eq!(replay.runs[0].observation.state.gpr[3], 0x1139)
            }
            PpuSequenceFamily::OverlappingMemory => {
                assert_eq!(
                    replay.runs[0].observation.memory[..4],
                    [0, 0x11, 0x62, 0x37]
                )
            }
            PpuSequenceFamily::StoreLoadForwarding => {
                assert_eq!(replay.runs[0].observation.state.gpr[6], 0x1137)
            }
            PpuSequenceFamily::Reservation => {
                assert!(replay.runs[0].observation.reservations.is_empty());
                assert_eq!(
                    replay.runs[0].observation.memory[..4],
                    0x1137u32.to_be_bytes()
                );
            }
            PpuSequenceFamily::ControlledBranch => {
                assert_eq!(&replay.runs[0].executed_pcs[..2], [0, 8]);
                assert_eq!(replay.runs[0].observation.state.gpr[3], 0x1137);
            }
            PpuSequenceFamily::Quickening => {
                assert_eq!(replay.runs[0].observation.state.gpr[3], 0x1138)
            }
            PpuSequenceFamily::FusionInvalidation => {
                assert_eq!(replay.runs[0].observation.state.gpr[3], 0x1136);
                assert_eq!(
                    replay.runs[0].observation.memory[..4],
                    0x1136u32.to_be_bytes()
                );
            }
        }
        assert!(first_path_divergence(&replay.runs).is_none());

        let mut defective = replay.runs;
        match family {
            PpuSequenceFamily::OverlappingMemory
            | PpuSequenceFamily::Reservation
            | PpuSequenceFamily::FusionInvalidation => {
                defective[3].observation.memory[0] ^= 1;
            }
            PpuSequenceFamily::RegisterAlias
            | PpuSequenceFamily::StoreLoadForwarding
            | PpuSequenceFamily::ControlledBranch
            | PpuSequenceFamily::Quickening => {
                let register = if family == PpuSequenceFamily::StoreLoadForwarding {
                    6
                } else {
                    3
                };
                defective[3].observation.state.gpr[register] ^= 1;
            }
        }
        let divergence = first_path_divergence(&defective).expect("seeded defect must diverge");
        assert_eq!(
            divergence.right,
            cellgov_fuzz::ppu_paths::PpuExecutionPath::Fused
        );
    }
}

#[test]
fn campaign_reports_intended_and_executed_opcode_bias_and_stop_causes() {
    let report = run_dependency_campaign(0x1137, PpuSequenceFamily::ALL.len() as u64)
        .expect("one pass over every family must run");

    assert_eq!(report.cases.len(), PpuSequenceFamily::ALL.len());
    assert!(PpuSequenceFamily::ALL
        .iter()
        .all(|family| report.intended_families.get(family) == Some(&1)));
    assert_eq!(
        report
            .stop_causes
            .get(&PpuSequenceStopClass::ControlTransfer),
        Some(&1)
    );
    assert!(report.intended_opcodes.values().sum::<u64>() > report.executed_opcodes.values().sum());
    let alias = &report.cases[0];
    assert_eq!(alias.intended_opcodes.len(), 1);
    assert_eq!(
        alias.intended_opcodes.values().copied().collect::<Vec<_>>(),
        [2]
    );
}

#[test]
fn exact_replay_retains_intent_eligibility_boundaries_and_trace_class() {
    let generated = generate_dependency_sequence(7, 4);
    let words = generated.words.clone();

    let replay = replay_dependency_sequence(generated).expect("branch family must replay");

    assert_eq!(replay.generated.family, PpuSequenceFamily::ControlledBranch);
    assert_eq!(replay.generated.words, words);
    assert_eq!(replay.stop, PpuSequenceStopClass::ControlTransfer);
    assert!(replay
        .generated
        .assessment
        .features
        .contains(&cellgov_fuzz::CaseFeature::ControlledFlow));
    assert_eq!(replay.generated.reduction_boundaries[0].end, words.len());
    assert_eq!(&replay.runs[0].executed_pcs[..2], [0, 8]);
}

#[test]
fn equal_seed_and_index_reproduce_the_same_structural_intent() {
    let first = generate_dependency_sequence(99, 3);
    let second = generate_dependency_sequence(99, 3);

    assert_eq!(first.family, second.family);
    assert_eq!(first.words, second.words);
    assert_eq!(first.data, second.data);
    assert_eq!(first.assessment, second.assessment);
    assert_eq!(first.reduction_boundaries, second.reduction_boundaries);
    assert_eq!(first.code_mutation, second.code_mutation);
}

#[test]
fn large_case_index_selects_a_family_before_narrowing() {
    let generated = generate_dependency_sequence(99, u64::MAX);
    let index = (u64::MAX % PpuSequenceFamily::ALL.len() as u64) as usize;
    assert_eq!(generated.family, PpuSequenceFamily::ALL[index]);
}

#[test]
fn an_out_of_range_code_mutation_is_refused() {
    let mut generated = generate_dependency_sequence(11, 6);
    generated
        .code_mutation
        .as_mut()
        .expect("mutation")
        .word_index = generated.words.len();

    assert!(matches!(
        replay_dependency_sequence(generated),
        Err(cellgov_fuzz::ppu_sequences::PpuSequenceCampaignError::Path(
            cellgov_fuzz::ppu_paths::PpuPathError::CodeMutationOutOfRange { .. }
        ))
    ));
}

#[test]
fn fusion_invalidation_intent_stales_both_slots_before_refresh() {
    let generated = generate_dependency_sequence(11, 6);
    let code = generated
        .words
        .iter()
        .flat_map(|word| word.to_be_bytes())
        .collect::<Vec<_>>();
    let mut shadow = cellgov_ppu::shadow::PredecodedShadow::build(0, &code);
    assert!(matches!(
        shadow.get(4),
        Some(cellgov_ppu::instruction::PpuInstruction::Consumed)
    ));
    let mutation = generated
        .code_mutation
        .expect("family must carry a rewrite");

    shadow.invalidate_range((mutation.word_index * 4) as u64, 4);

    assert!(shadow.get(0).is_none());
    assert!(shadow.get(4).is_none());
    assert!(shadow.refresh(0, mutation.replacement).is_some());
    assert!(shadow.get(0).is_some());
}
