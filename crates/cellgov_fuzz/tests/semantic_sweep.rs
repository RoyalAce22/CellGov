//! Independent kind lists expose gaps that a recipe registry cannot detect alone.

use cellgov_fuzz::semantic_sweep::{
    sweep_both, sweep_ppu, sweep_spu, SemanticCaseClass, SemanticEncoderTier,
    SemanticObservationTier, SemanticSweepFinding,
};
use cellgov_ppu::instruction::fuzz::generation_descriptors as ppu_descriptors;
use cellgov_spu::fuzz::generation_descriptors as spu_descriptors;

#[test]
fn both_interpreters_witness_every_declared_kind_and_structural_case_class() {
    let (ppu, spu) = sweep_both();
    for report in [&ppu, &spu] {
        assert_eq!(
            report.encoder_tier,
            SemanticEncoderTier::DescriptorRoundTrip
        );
        assert_eq!(
            report.observation_tier,
            SemanticObservationTier::DescriptorOnly
        );
        assert!(
            report.is_clean(),
            "{:?}",
            report.findings.iter().take(12).collect::<Vec<_>>()
        );
        assert!(report.expected_kinds.len() > 50);
        assert!(report.expected_refusals > 0);
        for class in [
            SemanticCaseClass::Canonical,
            SemanticCaseClass::RegisterAlias,
            SemanticCaseClass::OperandBoundary,
            SemanticCaseClass::ImmediateBoundary,
            SemanticCaseClass::ReservedField,
        ] {
            assert!(
                report
                    .witnesses
                    .iter()
                    .any(|witness| witness.classes.contains(&class)),
                "{class:?}"
            );
        }
    }
}

#[test]
fn a_removed_or_duplicated_recipe_is_a_named_coverage_defect() {
    let mut ppu = ppu_descriptors();
    let removed = ppu.pop().expect("registry must contain a kind");
    let absent = sweep_ppu(&ppu);
    assert!(absent
        .findings
        .contains(&SemanticSweepFinding::MissingKind {
            kind: cellgov_fuzz::InstructionIdentity::Ppu(removed.kind)
        }));
    assert!(!absent.is_clean());

    let mut spu = spu_descriptors();
    let duplicate = spu[0].clone();
    spu.push(duplicate.clone());
    let repeated = sweep_spu(&spu);
    assert!(repeated
        .findings
        .contains(&SemanticSweepFinding::DuplicateKind {
            kind: cellgov_fuzz::InstructionIdentity::Spu(duplicate.kind)
        }));
    assert!(!repeated.is_clean());
}

#[test]
fn rejected_and_ambiguous_words_have_distinct_diagnostic_classes() {
    let mut spu = spu_descriptors();
    let displaced = spu[0].kind;
    spu[0].canonical_word = u32::MAX;
    let rejected = sweep_spu(&spu);
    assert!(rejected.findings.iter().any(|finding| matches!(finding,
        SemanticSweepFinding::UnexpectedRejection { kind: cellgov_fuzz::InstructionIdentity::Spu(kind), raw: u32::MAX, .. }
        if *kind == displaced)));
    assert!(!rejected.is_clean());

    let mut ppu = ppu_descriptors();
    let shared = ppu[0].canonical_word;
    ppu[1].canonical_word = shared;
    let ambiguous = sweep_ppu(&ppu);
    assert!(ambiguous.findings.iter().any(|finding| matches!(finding,
        SemanticSweepFinding::AmbiguousWord { raw, .. } if *raw == shared)));
    assert!(!ambiguous.is_clean());
}

#[test]
fn a_recipe_that_erases_operands_or_invents_a_kind_cannot_claim_clean_coverage() {
    let mut spu = spu_descriptors();
    let index = spu
        .iter()
        .position(|descriptor| !descriptor.operands.is_empty())
        .expect("the SPU has operand-bearing instructions");
    let original = spu[index].clone();
    spu[index].operands.clear();
    let stripped = sweep_spu(&spu);
    assert!(stripped
        .findings
        .contains(&SemanticSweepFinding::DescriptorMismatch {
            kind: cellgov_fuzz::InstructionIdentity::Spu(original.kind),
            raw: original.canonical_word,
        }));
    assert!(!stripped.is_clean());

    let mut ppu = ppu_descriptors();
    ppu[0].kind = cellgov_ppu::instruction::fuzz::PpuFuzzKind::Ordinary(
        cellgov_ppu::instruction::PpuInstructionKind::Li,
    );
    let invented = sweep_ppu(&ppu);
    assert!(invented
        .findings
        .contains(&SemanticSweepFinding::UnexpectedKind {
            kind: cellgov_fuzz::InstructionIdentity::Ppu(ppu[0].kind),
        }));
    assert!(!invented.is_clean());
}
