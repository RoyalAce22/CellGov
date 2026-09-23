use super::*;

fn empty_report(kind: InstructionIdentity) -> SemanticSweepReport {
    SemanticSweepReport {
        encoder_tier: SemanticEncoderTier::DescriptorRoundTrip,
        observation_tier: SemanticObservationTier::DescriptorOnly,
        expected_kinds: BTreeSet::from([kind]),
        witnesses: Vec::new(),
        findings: BTreeSet::new(),
        expected_refusals: 0,
    }
}

#[test]
fn a_misencoded_case_is_distinct_from_an_unexpected_decoder_kind() {
    let kind = InstructionIdentity::Spu(
        *expected_spu_kinds()
            .iter()
            .next()
            .expect("SPU decoder declares kinds"),
    );
    let other = InstructionIdentity::Ppu(
        *expected_ppu_kinds()
            .iter()
            .next()
            .expect("PPU decoder declares kinds"),
    );
    let mut report = empty_report(kind);
    classify_candidate(
        kind,
        0x1234,
        SemanticCaseClass::Canonical,
        Some(kind),
        Some(0x1235),
        &mut report,
    );
    assert_eq!(
        report.findings,
        BTreeSet::from([SemanticSweepFinding::RoundTripFailure {
            kind,
            raw: 0x1234,
            encoded: Some(0x1235),
        }])
    );
    report.findings.clear();
    classify_candidate(
        kind,
        0x1234,
        SemanticCaseClass::Canonical,
        Some(other),
        Some(0x1234),
        &mut report,
    );
    assert_eq!(
        report.findings,
        BTreeSet::from([SemanticSweepFinding::Misclassified {
            expected: kind,
            actual: other,
            raw: 0x1234,
        }])
    );
}

#[test]
fn a_structurally_invalid_operand_that_encodes_is_an_unexpected_acceptance() {
    let kind = InstructionIdentity::Spu(
        *expected_spu_kinds()
            .iter()
            .next()
            .expect("SPU decoder declares kinds"),
    );
    let words = BTreeMap::from([(0x1234, BTreeSet::from([SemanticCaseClass::Canonical]))]);
    let report = sweep_descriptors(
        BTreeSet::from([kind]),
        [Ok((
            kind,
            words,
            vec![InvalidProbe::Accepted {
                field: 2,
                raw: 0x4567,
            }],
        ))]
        .into_iter(),
        |_| Some(kind),
        Some,
    );
    assert!(report
        .findings
        .contains(&SemanticSweepFinding::UnexpectedAcceptance {
            kind,
            field: 2,
            raw: 0x4567,
        }));
    assert_eq!(report.witnesses.len(), 1);
    assert!(!report.is_clean());
}

#[test]
fn target_and_generation_panics_remain_named_findings() {
    let kind = InstructionIdentity::Spu(
        *expected_spu_kinds()
            .iter()
            .next()
            .expect("SPU decoder declares kinds"),
    );
    let words = || BTreeMap::from([(0x1234, BTreeSet::from([SemanticCaseClass::Canonical]))]);
    let generated = || [Ok((kind, words(), vec![]))].into_iter();
    let decode_panic = sweep_descriptors(
        BTreeSet::from([kind]),
        generated(),
        |_| -> Option<InstructionIdentity> {
            panic!("seeded decoder fault");
        },
        Some,
    );
    assert!(decode_panic.findings.iter().any(|finding| matches!(finding,
        SemanticSweepFinding::TargetPanic { kind: found, raw: 0x1234, stage: SemanticTargetStage::Decoder, .. }
        if *found == kind)));
    assert!(!decode_panic.is_clean());

    let encode_panic = sweep_descriptors(
        BTreeSet::from([kind]),
        generated(),
        |_| Some(kind),
        |_| -> Option<u32> {
            panic!("seeded encoder fault");
        },
    );
    assert!(encode_panic.findings.iter().any(|finding| matches!(finding,
        SemanticSweepFinding::TargetPanic { kind: found, raw: 0x1234, stage: SemanticTargetStage::DescriptorEncoder, .. }
        if *found == kind)));

    let generation_panic = sweep_descriptors(
        BTreeSet::from([kind]),
        [Err((kind, TargetPanicPayload::NonString))].into_iter(),
        |_| Some(kind),
        Some,
    );
    assert!(generation_panic
        .findings
        .contains(&SemanticSweepFinding::GenerationPanic {
            kind,
            payload: TargetPanicPayload::NonString,
        }));
    assert!(generation_panic
        .findings
        .contains(&SemanticSweepFinding::UnwitnessedKind { kind }));
}

#[test]
fn every_bounded_operand_is_probed_and_decoder_filtering_cannot_hide_a_boundary() {
    let descriptor = spu_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.operands.len() >= 2)
        .expect("SPU has a multi-operand descriptor");
    let probes = out_of_range_spu(&descriptor);
    assert_eq!(
        probes.len(),
        descriptor
            .operands
            .iter()
            .filter(|field| field.maximum() < u32::MAX)
            .count()
    );
    assert!(probes.iter().all(|probe| *probe == InvalidProbe::Refused));

    let kind = InstructionIdentity::Spu(descriptor.kind);
    let cases = spu_cases(&descriptor);
    let boundary = *cases
        .iter()
        .find(|(_, classes)| classes.contains(&SemanticCaseClass::OperandBoundary))
        .expect("descriptor has a packed boundary")
        .0;
    let report = sweep_descriptors(
        BTreeSet::from([kind]),
        [Ok((kind, cases, probes))].into_iter(),
        |raw| (raw != boundary).then_some(kind),
        Some,
    );
    assert!(report.findings.iter().any(|finding| matches!(finding,
        SemanticSweepFinding::UnexpectedRejection { kind: found, raw, class: SemanticCaseClass::OperandBoundary }
        if *found == kind && *raw == boundary)));
}
