use std::collections::BTreeSet;

use cellgov_spu::exec::SpuStepOutcome;
use cellgov_spu::observation::{SpuAllowedFootprint, SpuObservation, SpuObservationComponent};
use cellgov_spu::state::SpuState;

use super::*;
use crate::artifact::ArtifactFingerprint;
use crate::retention::RetentionConfig;
use crate::GenerationStrategy;

/// A footprint that allows no write, so every changed component is a violation.
fn sealed() -> SpuAllowedFootprint {
    SpuAllowedFootprint {
        registers: BTreeSet::new(),
        local_store: false,
        channels: BTreeSet::new(),
        reservation: false,
        control_transfer: false,
        fpscr: false,
        signals: false,
        interrupts: false,
        effects: BTreeSet::new(),
    }
}

#[test]
fn two_stray_writes_are_two_finding_identities() {
    let initial = SpuState::new();
    let mut after = SpuState::new();
    after.set_reg(3, [1; 16]);
    after.ls[0] = 1;
    let violations = sealed().violations(
        &initial,
        &SpuObservation::capture(&after, &SpuStepOutcome::Continue),
    );
    assert_eq!(
        violations,
        BTreeSet::from([
            SpuObservationComponent::Registers,
            SpuObservationComponent::LocalStore,
        ])
    );

    let mut report = FuzzReport::new(
        FuzzTarget::SpuInstruction,
        7,
        GenerationStrategy::Structured,
        RetentionConfig::default(),
        2,
        1,
    );
    for component in &violations {
        record(
            &mut report,
            FindingKind::IllegalFootprint,
            footprint_fingerprint(FuzzTarget::SpuInstruction, None, *component, None),
            vec![0],
            0,
        )
        .expect("recorded");
    }

    let identities: BTreeSet<SemanticFingerprint> = report
        .findings
        .iter()
        .map(|finding| finding.fingerprint)
        .collect();
    assert_eq!(identities.len(), 2);
    assert_eq!(
        report
            .findings
            .iter()
            .map(|finding| finding.fingerprint.component)
            .collect::<Vec<_>>(),
        [
            Some(ComponentIdentity::Spu(SpuObservationComponent::Registers)),
            Some(ComponentIdentity::Spu(SpuObservationComponent::LocalStore)),
        ]
    );
}

#[test]
fn the_component_rides_the_artifact_fingerprint() {
    let fingerprint = footprint_fingerprint(
        FuzzTarget::SpuSequence,
        None,
        SpuObservationComponent::Channels,
        None,
    );
    assert_eq!(fingerprint.check, CheckIdentity::AllowedFootprint);
    assert_eq!(fingerprint.divergence, DivergenceClass::ArchitecturalState);
    assert_eq!(
        ArtifactFingerprint::from(&fingerprint).component.as_deref(),
        Some("Spu(Channels)")
    );
}
