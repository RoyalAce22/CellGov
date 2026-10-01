use super::*;
use cellgov_spu::exec::SpuStepOutcome;

use crate::report::ComponentIdentity;

/// `a $3,$4,$5`: the commutative relation executes against `baseline`.
const ADD_WORD: u32 = 0x1800_0000 | (5 << 14) | (4 << 7) | 3;

fn add_initial() -> SpuState {
    let mut initial = SpuState::new();
    initial.set_reg(4, [0x11; 16]);
    initial.set_reg(5, [0x22; 16]);
    initial
}

fn add_run(baseline: &ObservedStep) -> (FuzzReport, CrossReferenceAsymmetry) {
    let instruction = cellgov_spu::decode::decode(ADD_WORD).unwrap();
    let descriptor = instruction.fuzz_descriptor();
    let config = FuzzConfig::default();
    let mut report = FuzzReport::new(
        FuzzTarget::SpuInstruction,
        config.seed,
        config.strategy,
        config.retention,
        config.max_findings as usize,
        config.sequence_words,
    );
    let asymmetry = run_metamorphic_checks(
        &mut report,
        &instruction,
        descriptor,
        &add_initial(),
        baseline,
        ADD_WORD,
        InstructionIdentity::Spu(descriptor.kind),
        0,
    )
    .unwrap();
    (report, asymmetry)
}

fn add_baseline() -> ObservedStep {
    let instruction = cellgov_spu::decode::decode(ADD_WORD).unwrap();
    run_once(&instruction, &add_initial())
}

#[test]
fn a_relation_finding_names_the_component_that_decided_its_class() {
    // Premise: an unperturbed baseline executes the relation cleanly.
    let clean = add_baseline();
    let (report, asymmetry) = add_run(&clean);
    assert_eq!(asymmetry, CrossReferenceAsymmetry::None);
    let executed: u64 = report.metamorphic_executions.values().sum();
    assert!(executed > 0, "{:?}", report.metamorphic_executions);
    assert!(report.findings.is_empty());

    // A register the relation never permits to move is a Registers difference.
    let mut registers = clean.clone();
    registers.state.regs[3][15] ^= 1;
    let (report, asymmetry) = add_run(&registers);
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
            Some(ComponentIdentity::Spu(SpuObservationComponent::Registers)),
            "{finding:?}"
        );
    }

    // A differing terminal outcome names Outcome, not the state fallback.
    let mut outcome = clean;
    outcome.outcome = SpuStepOutcome::Branch;
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
            Some(ComponentIdentity::Spu(SpuObservationComponent::Outcome)),
            "{finding:?}"
        );
    }
}

#[test]
fn the_deciding_component_outranks_the_rest_of_the_difference_set() {
    let mixed = BTreeSet::from([
        SpuObservationComponent::Registers,
        SpuObservationComponent::ProgramCounter,
        SpuObservationComponent::Outcome,
        SpuObservationComponent::Effects,
        SpuObservationComponent::FaultDiscard,
    ]);
    assert_eq!(
        metamorphic_divergence(&mixed),
        Some((
            SpuObservationComponent::FaultDiscard,
            CrossReferenceAsymmetry::Fault
        ))
    );
    let mut rest = mixed;
    for expected in [
        (
            SpuObservationComponent::Effects,
            CrossReferenceAsymmetry::Effect,
        ),
        (
            SpuObservationComponent::Outcome,
            CrossReferenceAsymmetry::Outcome,
        ),
        (
            SpuObservationComponent::ProgramCounter,
            CrossReferenceAsymmetry::State,
        ),
        (
            SpuObservationComponent::Registers,
            CrossReferenceAsymmetry::State,
        ),
    ] {
        let (decided, _) = metamorphic_divergence(&rest).unwrap();
        rest.remove(&decided);
        assert_eq!(metamorphic_divergence(&rest), Some(expected));
    }
    // The state fallback names the lowest component in observation order.
    let state = BTreeSet::from([
        SpuObservationComponent::Fpscr,
        SpuObservationComponent::LocalStore,
    ]);
    assert_eq!(
        metamorphic_divergence(&state),
        Some((
            SpuObservationComponent::LocalStore,
            CrossReferenceAsymmetry::State
        ))
    );
    assert_eq!(metamorphic_divergence(&BTreeSet::new()), None);
}
