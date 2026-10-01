use super::*;
use cellgov_ppu::instruction::fuzz::{generation_descriptors, PpuFuzzKind};
use cellgov_ppu::instruction::PpuInstructionKind;
use cellgov_ppu::observation::PpuObservedOutcome;

use crate::report::ComponentIdentity;

/// The `add` case with its record and overflow controls clear, so both of
/// its relations execute against `baseline`.
fn add_run(baseline: &ObservedStep) -> (FuzzReport, CrossReferenceAsymmetry) {
    let generation = generation_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.kind == PpuFuzzKind::Ordinary(PpuInstructionKind::Add))
        .unwrap();
    let raw = generation.canonical_word & !0x0000_0401;
    let instruction = cellgov_ppu::decode::decode(raw).unwrap();
    let descriptor = instruction.fuzz_descriptor(raw);
    let initial = PpuState::new();
    let memory = vec![0; DATA_LEN];
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
            baseline,
        },
    )
    .unwrap();
    (report, asymmetry)
}

fn add_baseline() -> ObservedStep {
    let generation = generation_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.kind == PpuFuzzKind::Ordinary(PpuInstructionKind::Add))
        .unwrap();
    let raw = generation.canonical_word & !0x0000_0401;
    let instruction = cellgov_ppu::decode::decode(raw).unwrap();
    let memory = vec![0; DATA_LEN];
    run_once(&instruction, &PpuState::new(), &memory).unwrap()
}

#[test]
fn a_relation_finding_names_the_component_that_decided_its_class() {
    // Premise: an unperturbed baseline executes both relations cleanly.
    let clean = add_baseline();
    let (report, asymmetry) = add_run(&clean);
    assert_eq!(asymmetry, CrossReferenceAsymmetry::None);
    let executed: u64 = report.metamorphic_executions.values().sum();
    assert!(executed > 0, "{:?}", report.metamorphic_executions);
    assert!(report.findings.is_empty());

    // A register the relation never permits to move is a State difference.
    let mut state = clean.clone();
    state.observation.state.gpr[0] ^= 1;
    let (report, asymmetry) = add_run(&state);
    assert_eq!(asymmetry, CrossReferenceAsymmetry::State);
    assert_eq!(
        report
            .finding_counts
            .get(&FindingKind::MetamorphicViolation),
        Some(&executed)
    );
    for finding in &report.findings {
        assert_eq!(
            finding.fingerprint.divergence,
            DivergenceClass::ArchitecturalState,
            "{finding:?}"
        );
        assert_eq!(
            finding.fingerprint.component,
            Some(ComponentIdentity::Ppu(PpuObservationComponent::State)),
            "{finding:?}"
        );
    }

    // A differing terminal outcome names Outcome, not the state fallback.
    let mut outcome = clean;
    outcome.observation.outcome = PpuObservedOutcome::Execution(ExecuteVerdict::Branch);
    let (report, asymmetry) = add_run(&outcome);
    assert_eq!(asymmetry, CrossReferenceAsymmetry::Outcome);
    assert_eq!(
        report
            .finding_counts
            .get(&FindingKind::MetamorphicViolation),
        Some(&executed)
    );
    for finding in &report.findings {
        assert_eq!(
            finding.fingerprint.divergence,
            DivergenceClass::Outcome,
            "{finding:?}"
        );
        assert_eq!(
            finding.fingerprint.component,
            Some(ComponentIdentity::Ppu(PpuObservationComponent::Outcome)),
            "{finding:?}"
        );
    }
}
