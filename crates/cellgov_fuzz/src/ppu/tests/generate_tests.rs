use super::*;
use cellgov_ppu::instruction::fuzz::{PpuFuzzKind, PpuSequenceDependency};
use cellgov_ppu::instruction::PpuInstructionKind;

use crate::ppu::execute::run_once;

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
