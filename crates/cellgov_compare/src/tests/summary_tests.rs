//! Cross-runner summary derivation: convergence-failure priority, byte-parity classes, and validate invariants.

use super::*;
use crate::observation::{NamedMemoryRegion, Observation, ObservationMetadata};
use crate::observation_compare::{compare_observations, ByteDivergence};

fn obs(
    outcome: ObservedOutcome,
    regions: Vec<NamedMemoryRegion>,
    runner: &str,
    steps: Option<usize>,
) -> Observation {
    Observation {
        outcome,
        memory_regions: regions,
        events: Vec::new(),
        state_hashes: None,
        metadata: ObservationMetadata {
            runner: runner.to_string(),
            steps,
        },
        tty_log: Vec::new(),
    }
}

fn region(name: &str, addr: u64, data: Vec<u8>) -> NamedMemoryRegion {
    NamedMemoryRegion {
        name: name.to_string(),
        addr,
        data,
    }
}

#[test]
fn identical_observations_yield_equivalent_byte_parity() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("r", 0, vec![0u8; 4])],
        "cellgov",
        Some(1),
    );
    let b = a.clone();
    let s = summarize(&compare_observations(&a, &b), &[]);
    assert_eq!(s.convergence, Convergence::Yes);
    assert_eq!(s.byte_parity, ByteParity::Equivalent);
    let (conv, parity) = s.display_matrix_columns();
    assert_eq!(conv, "Yes");
    assert_eq!(parity, "equivalent");
}

#[test]
fn all_non_semantic_classifies_as_non_semantic_byte_parity() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0u8; 0x40])],
        "cellgov",
        Some(1),
    );
    let mut b_data = vec![0u8; 0x40];
    b_data[0x17] = 0x01;
    b_data[0x35] = 0x40;
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, b_data)],
        "rpcs3",
        Some(1),
    );
    let result = compare_observations(&a, &b);
    let classes = vec![DivergenceClass::ElfHeader, DivergenceClass::ElfHeader];
    let s = summarize(&result, &classes);
    assert_eq!(s.convergence, Convergence::Yes);
    assert_eq!(s.byte_parity, ByteParity::NonSemantic { bytes: 2 });
    let (conv, parity) = s.display_matrix_columns();
    assert_eq!(conv, "Yes");
    assert_eq!(parity, "2 non-semantic");
}

#[test]
fn non_semantic_renders_consistent_form_at_count_one() {
    // Symmetric with Pending's bare-count form: no singular case.
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0u8; 0x40])],
        "cellgov",
        Some(1),
    );
    let mut b_data = vec![0u8; 0x40];
    b_data[0x17] = 0x01;
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, b_data)],
        "rpcs3",
        Some(1),
    );
    let s = summarize(&compare_observations(&a, &b), &[DivergenceClass::ElfHeader]);
    let (_, parity) = s.display_matrix_columns();
    assert_eq!(parity, "1 non-semantic");
}

#[test]
fn mixed_classes_yields_pending_byte_parity_with_split_counts() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![
            region("code", 0x10000, vec![0u8; 0x40]),
            region("data", 0x80000, vec![0u8; 8]),
        ],
        "cellgov",
        Some(1),
    );
    let mut b_code = vec![0u8; 0x40];
    b_code[0x17] = 0x01;
    let b_data = vec![0xFFu8; 8];
    let b = obs(
        ObservedOutcome::Completed,
        vec![
            region("code", 0x10000, b_code),
            region("data", 0x80000, b_data),
        ],
        "rpcs3",
        Some(1),
    );
    let s = summarize(
        &compare_observations(&a, &b),
        &[DivergenceClass::ElfHeader, DivergenceClass::Unclassified],
    );
    assert_eq!(s.convergence, Convergence::Yes);
    assert_eq!(
        s.byte_parity,
        ByteParity::Pending {
            non_semantic_bytes: 1,
            unclassified_bytes: 8,
        }
    );
    let (conv, parity) = s.display_matrix_columns();
    assert_eq!(conv, "Yes");
    assert_eq!(parity, "1 non-semantic + 8 pending");
}

#[test]
fn outcome_mismatch_yields_no_convergence_and_diverge_byte_parity() {
    let a = obs(
        ObservedOutcome::Fault,
        vec![region("r", 0, vec![0])],
        "cellgov",
        Some(1),
    );
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("r", 0, vec![0])],
        "rpcs3",
        Some(1),
    );
    let s = summarize(&compare_observations(&a, &b), &[]);
    match (&s.convergence, &s.byte_parity) {
        (
            Convergence::No {
                reason: ConvergenceFailure::OutcomeMismatch { cellgov, rpcs3 },
            },
            ByteParity::Diverge { reason },
        ) => {
            assert_eq!(*cellgov, ObservedOutcome::Fault);
            assert_eq!(*rpcs3, ObservedOutcome::Completed);
            assert!(matches!(reason, ConvergenceFailure::OutcomeMismatch { .. }));
        }
        other => panic!("unexpected: {other:?}"),
    }
    let (conv, parity) = s.display_matrix_columns();
    assert_eq!(conv, "No (outcome: Fault vs Completed)");
    assert_eq!(parity, "--");
    assert!(s.per_class_bytes.is_empty());
    assert!(s.unclassified_runs.is_empty());
    assert!(s.lowest_offset_class.is_none());
}

#[test]
fn region_count_mismatch_yields_no_convergence() {
    let a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("r", 0, vec![0])],
        "rpcs3",
        Some(1),
    );
    let s = summarize(&compare_observations(&a, &b), &[]);
    assert!(matches!(
        s.convergence,
        Convergence::No {
            reason: ConvergenceFailure::RegionCountMismatch {
                cellgov: 0,
                rpcs3: 1
            }
        }
    ));
}

#[test]
fn region_identity_mismatch_yields_no_convergence() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0u8; 4])],
        "cellgov",
        Some(1),
    );
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("data", 0x80000, vec![0u8; 4])],
        "rpcs3",
        Some(1),
    );
    let s = summarize(&compare_observations(&a, &b), &[]);
    match s.convergence {
        Convergence::No {
            reason:
                ConvergenceFailure::RegionIdentityMismatch {
                    index,
                    cellgov,
                    rpcs3,
                },
        } => {
            assert_eq!(index, 0);
            assert_eq!(cellgov.name, "code");
            assert_eq!(cellgov.addr, 0x10000);
            assert_eq!(rpcs3.name, "data");
            assert_eq!(rpcs3.addr, 0x80000);
        }
        other => panic!("expected RegionIdentityMismatch, got {other:?}"),
    }
}

#[test]
fn same_runner_step_mismatch_yields_no_convergence() {
    let a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(100));
    let b = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(200));
    let s = summarize(&compare_observations(&a, &b), &[]);
    match s.convergence {
        Convergence::No {
            reason: ConvergenceFailure::SameRunnerStepMismatch { runner, a, b },
        } => {
            assert_eq!(runner, "cellgov");
            assert_eq!(a, 100);
            assert_eq!(b, 200);
        }
        other => panic!("expected SameRunnerStepMismatch, got {other:?}"),
    }
}

#[test]
fn cross_runner_step_delta_yields_no_convergence() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("r", 0, vec![0u8; 4])],
        "cellgov",
        Some(100),
    );
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("r", 0, vec![0u8; 4])],
        "rpcs3",
        Some(101),
    );
    let s = summarize(&compare_observations(&a, &b), &[]);
    match s.convergence {
        Convergence::No {
            reason: ConvergenceFailure::CrossRunnerStepMismatch { cellgov, rpcs3 },
        } => {
            assert_eq!(cellgov, 100);
            assert_eq!(rpcs3, 101);
        }
        other => panic!("expected CrossRunnerStepMismatch, got {other:?}"),
    }
}

#[test]
fn lowest_offset_class_picks_by_guest_start() {
    let a_code = vec![0u8; 0x40];
    let mut b_code = vec![0u8; 0x40];
    b_code[0x35] = 0x40;
    let a_data = vec![0u8; 0x20];
    let mut b_data = vec![0u8; 0x20];
    b_data[0x10] = 0xAA;
    let a = obs(
        ObservedOutcome::Completed,
        vec![
            region("code", 0x10000, a_code),
            region("data", 0x80000, a_data),
        ],
        "cellgov",
        Some(1),
    );
    let b = obs(
        ObservedOutcome::Completed,
        vec![
            region("code", 0x10000, b_code),
            region("data", 0x80000, b_data),
        ],
        "rpcs3",
        Some(1),
    );
    let s = summarize(
        &compare_observations(&a, &b),
        &[DivergenceClass::ElfHeader, DivergenceClass::HleOpdSlot],
    );
    let (cls, ident, off) = s.lowest_offset_class.unwrap();
    assert_eq!(cls, DivergenceClass::ElfHeader);
    assert_eq!(ident.name, "code");
    assert_eq!(ident.addr, 0x10000);
    assert_eq!(off, 0x35);
}

#[test]
fn per_class_bytes_iterates_in_discriminant_order() {
    let a_code = vec![0u8; 0x40];
    let mut b_code = vec![0u8; 0x40];
    b_code[0x10] = 0xAA;
    let a_data = vec![0u8; 0x20];
    let mut b_data = vec![0u8; 0x20];
    b_data[0x00] = 0xBB;
    b_data[0x01] = 0xCC;
    let a = obs(
        ObservedOutcome::Completed,
        vec![
            region("code", 0x10000, a_code),
            region("data", 0x80000, a_data),
        ],
        "cellgov",
        Some(1),
    );
    let b = obs(
        ObservedOutcome::Completed,
        vec![
            region("code", 0x10000, b_code),
            region("data", 0x80000, b_data),
        ],
        "rpcs3",
        Some(1),
    );
    let s = summarize(
        &compare_observations(&a, &b),
        &[DivergenceClass::ElfHeader, DivergenceClass::HleOpdSlot],
    );
    let keys: Vec<DivergenceClass> = s.per_class_bytes.keys().copied().collect();
    assert_eq!(
        keys,
        vec![DivergenceClass::ElfHeader, DivergenceClass::HleOpdSlot]
    );
    assert_eq!(s.per_class_bytes[&DivergenceClass::ElfHeader], 1);
    assert_eq!(s.per_class_bytes[&DivergenceClass::HleOpdSlot], 2);
}

#[test]
#[should_panic(expected = "shorter than the byte-divergence count")]
fn mismatched_classes_length_panics_short() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("r", 0, vec![0u8; 2])],
        "cellgov",
        Some(1),
    );
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("r", 0, vec![0xFF; 2])],
        "rpcs3",
        Some(1),
    );
    summarize(&compare_observations(&a, &b), &[]);
}

#[test]
#[should_panic(expected = "longer than the byte-divergence count")]
fn mismatched_classes_length_panics_long() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("r", 0, vec![0u8; 2])],
        "cellgov",
        Some(1),
    );
    let b = a.clone();
    // No byte divergences, but a non-empty classes vector.
    summarize(&compare_observations(&a, &b), &[DivergenceClass::ElfHeader]);
}

#[test]
fn region_length_mismatch_yields_no_convergence() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0u8; 4])],
        "cellgov",
        Some(1),
    );
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0u8; 8])],
        "rpcs3",
        Some(1),
    );
    let s = summarize(&compare_observations(&a, &b), &[]);
    match s.convergence {
        Convergence::No {
            reason:
                ConvergenceFailure::RegionLengthMismatch {
                    index,
                    name,
                    cellgov_length,
                    rpcs3_length,
                },
        } => {
            assert_eq!(index, 0);
            assert_eq!(name, "code");
            assert_eq!(cellgov_length, 4);
            assert_eq!(rpcs3_length, 8);
        }
        other => panic!("expected RegionLengthMismatch, got {other:?}"),
    }
}

#[test]
fn outcome_mismatch_wins_over_region_count_mismatch() {
    // Both outcomes differ AND region counts differ -> outcome
    // wins the priority chain.
    let a = obs(ObservedOutcome::Fault, vec![], "cellgov", Some(1));
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("r", 0, vec![0])],
        "rpcs3",
        Some(1),
    );
    let s = summarize(&compare_observations(&a, &b), &[]);
    assert!(
        matches!(
            s.convergence,
            Convergence::No {
                reason: ConvergenceFailure::OutcomeMismatch { .. }
            }
        ),
        "outcome mismatch wins; got {:?}",
        s.convergence
    );
}

#[test]
fn region_count_mismatch_wins_over_region_identity_mismatch() {
    // Region count differs (so identity-mismatch is unreachable
    // because pairs is empty) AND outcome agrees -> count wins.
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0])],
        "cellgov",
        Some(1),
    );
    let b = obs(
        ObservedOutcome::Completed,
        vec![
            region("data", 0x80000, vec![0]),
            region("extra", 0x90000, vec![0]),
        ],
        "rpcs3",
        Some(1),
    );
    let s = summarize(&compare_observations(&a, &b), &[]);
    assert!(matches!(
        s.convergence,
        Convergence::No {
            reason: ConvergenceFailure::RegionCountMismatch { .. }
        }
    ));
}

fn outcome_mismatch_reason() -> ConvergenceFailure {
    ConvergenceFailure::OutcomeMismatch {
        cellgov: ObservedOutcome::Fault,
        rpcs3: ObservedOutcome::Completed,
    }
}

fn empty_diverged(reason: ConvergenceFailure) -> CrossRunnerSummary {
    CrossRunnerSummary {
        convergence: Convergence::No {
            reason: reason.clone(),
        },
        byte_parity: ByteParity::Diverge { reason },
        per_class_bytes: BTreeMap::new(),
        unclassified_bytes: 0,
        unclassified_runs: Vec::new(),
        lowest_offset_class: None,
    }
}

fn empty_converged_equivalent() -> CrossRunnerSummary {
    CrossRunnerSummary {
        convergence: Convergence::Yes,
        byte_parity: ByteParity::Equivalent,
        per_class_bytes: BTreeMap::new(),
        unclassified_bytes: 0,
        unclassified_runs: Vec::new(),
        lowest_offset_class: None,
    }
}

#[test]
fn validate_accepts_canonical_converged_equivalent() {
    empty_converged_equivalent().validate().unwrap();
}

#[test]
fn validate_accepts_canonical_diverged() {
    empty_diverged(outcome_mismatch_reason())
        .validate()
        .unwrap();
}

#[test]
fn validate_rejects_converged_with_diverge_byte_parity() {
    let bad = CrossRunnerSummary {
        convergence: Convergence::Yes,
        byte_parity: ByteParity::Diverge {
            reason: outcome_mismatch_reason(),
        },
        per_class_bytes: BTreeMap::new(),
        unclassified_bytes: 0,
        unclassified_runs: Vec::new(),
        lowest_offset_class: None,
    };
    assert!(matches!(
        bad.validate().unwrap_err(),
        CrossRunnerSummaryError::ConvergedButByteParityDiverged
    ));
}

#[test]
fn validate_rejects_diverged_without_diverge_byte_parity() {
    let bad = CrossRunnerSummary {
        convergence: Convergence::No {
            reason: outcome_mismatch_reason(),
        },
        byte_parity: ByteParity::Equivalent,
        per_class_bytes: BTreeMap::new(),
        unclassified_bytes: 0,
        unclassified_runs: Vec::new(),
        lowest_offset_class: None,
    };
    assert!(matches!(
        bad.validate().unwrap_err(),
        CrossRunnerSummaryError::DivergedButByteParityNotDiverge
    ));
}

#[test]
fn validate_rejects_diverge_reasons_disagree() {
    let bad = CrossRunnerSummary {
        convergence: Convergence::No {
            reason: outcome_mismatch_reason(),
        },
        byte_parity: ByteParity::Diverge {
            reason: ConvergenceFailure::RegionCountMismatch {
                cellgov: 1,
                rpcs3: 2,
            },
        },
        per_class_bytes: BTreeMap::new(),
        unclassified_bytes: 0,
        unclassified_runs: Vec::new(),
        lowest_offset_class: None,
    };
    assert!(matches!(
        bad.validate().unwrap_err(),
        CrossRunnerSummaryError::DivergeReasonsDisagree
    ));
}

#[test]
fn validate_rejects_diverged_with_non_empty_per_class_bytes() {
    let mut bad = empty_diverged(outcome_mismatch_reason());
    bad.per_class_bytes.insert(DivergenceClass::ElfHeader, 1);
    assert!(matches!(
        bad.validate().unwrap_err(),
        CrossRunnerSummaryError::DivergedButPerClassNonEmpty
    ));
}

#[test]
fn validate_rejects_diverged_with_non_zero_unclassified_bytes() {
    let mut bad = empty_diverged(outcome_mismatch_reason());
    bad.unclassified_bytes = 1;
    assert!(matches!(
        bad.validate().unwrap_err(),
        CrossRunnerSummaryError::DivergedButUnclassifiedBytesNonZero
    ));
}

#[test]
fn validate_rejects_diverged_with_non_empty_unclassified_runs() {
    let mut bad = empty_diverged(outcome_mismatch_reason());
    bad.unclassified_runs.push(UnclassifiedRun {
        region_name: "r".to_string(),
        offset: 0,
        length: 1,
    });
    assert!(matches!(
        bad.validate().unwrap_err(),
        CrossRunnerSummaryError::DivergedButUnclassifiedRunsNonEmpty
    ));
}

#[test]
fn validate_rejects_diverged_with_lowest_offset_some() {
    let mut bad = empty_diverged(outcome_mismatch_reason());
    bad.lowest_offset_class = Some((
        DivergenceClass::ElfHeader,
        RegionIdent {
            name: "r".to_string(),
            addr: 0,
        },
        0,
    ));
    assert!(matches!(
        bad.validate().unwrap_err(),
        CrossRunnerSummaryError::DivergedButLowestOffsetSome
    ));
}

#[test]
fn validate_rejects_unclassified_denormalization_mismatch() {
    let bad = CrossRunnerSummary {
        convergence: Convergence::Yes,
        byte_parity: ByteParity::Pending {
            non_semantic_bytes: 0,
            unclassified_bytes: 10,
        },
        per_class_bytes: BTreeMap::from([(DivergenceClass::Unclassified, 5)]),
        unclassified_bytes: 10,
        unclassified_runs: vec![UnclassifiedRun {
            region_name: "r".to_string(),
            offset: 0,
            length: 10,
        }],
        lowest_offset_class: None,
    };
    assert!(matches!(
        bad.validate().unwrap_err(),
        CrossRunnerSummaryError::UnclassifiedDenormalizationMismatch {
            per_class_unclassified: 5,
            unclassified_bytes_field: 10,
        }
    ));
}

#[test]
fn validate_rejects_unclassified_runs_sum_mismatch() {
    let bad = CrossRunnerSummary {
        convergence: Convergence::Yes,
        byte_parity: ByteParity::Pending {
            non_semantic_bytes: 0,
            unclassified_bytes: 10,
        },
        per_class_bytes: BTreeMap::from([(DivergenceClass::Unclassified, 10)]),
        unclassified_bytes: 10,
        unclassified_runs: vec![UnclassifiedRun {
            region_name: "r".to_string(),
            offset: 0,
            length: 7,
        }],
        lowest_offset_class: None,
    };
    assert!(matches!(
        bad.validate().unwrap_err(),
        CrossRunnerSummaryError::UnclassifiedRunsSumMismatch {
            runs_length_sum: 7,
            unclassified_bytes_field: 10,
        }
    ));
}

#[test]
fn validate_rejects_equivalent_with_non_zero_totals() {
    let bad = CrossRunnerSummary {
        convergence: Convergence::Yes,
        byte_parity: ByteParity::Equivalent,
        per_class_bytes: BTreeMap::from([(DivergenceClass::ElfHeader, 1)]),
        unclassified_bytes: 0,
        unclassified_runs: Vec::new(),
        lowest_offset_class: None,
    };
    assert!(matches!(
        bad.validate().unwrap_err(),
        CrossRunnerSummaryError::ByteParityEquivalentButTotalsNonZero
    ));
}

#[test]
fn validate_rejects_non_semantic_bytes_disagreement() {
    let bad = CrossRunnerSummary {
        convergence: Convergence::Yes,
        byte_parity: ByteParity::NonSemantic { bytes: 5 },
        per_class_bytes: BTreeMap::from([(DivergenceClass::ElfHeader, 3)]),
        unclassified_bytes: 0,
        unclassified_runs: Vec::new(),
        lowest_offset_class: None,
    };
    assert!(matches!(
        bad.validate().unwrap_err(),
        CrossRunnerSummaryError::ByteParityNonSemanticBytesMismatch {
            variant: 5,
            computed: 3,
        }
    ));
}

#[test]
fn validate_rejects_non_semantic_with_unclassified_present() {
    let bad = CrossRunnerSummary {
        convergence: Convergence::Yes,
        byte_parity: ByteParity::NonSemantic { bytes: 3 },
        per_class_bytes: BTreeMap::from([
            (DivergenceClass::ElfHeader, 3),
            (DivergenceClass::Unclassified, 2),
        ]),
        unclassified_bytes: 2,
        unclassified_runs: vec![UnclassifiedRun {
            region_name: "r".to_string(),
            offset: 0,
            length: 2,
        }],
        lowest_offset_class: None,
    };
    assert!(matches!(
        bad.validate().unwrap_err(),
        CrossRunnerSummaryError::ByteParityNonSemanticButUnclassifiedNonZero
    ));
}

#[test]
fn validate_rejects_pending_non_semantic_disagreement() {
    let bad = CrossRunnerSummary {
        convergence: Convergence::Yes,
        byte_parity: ByteParity::Pending {
            non_semantic_bytes: 10,
            unclassified_bytes: 2,
        },
        per_class_bytes: BTreeMap::from([
            (DivergenceClass::ElfHeader, 3),
            (DivergenceClass::Unclassified, 2),
        ]),
        unclassified_bytes: 2,
        unclassified_runs: vec![UnclassifiedRun {
            region_name: "r".to_string(),
            offset: 0,
            length: 2,
        }],
        lowest_offset_class: None,
    };
    assert!(matches!(
        bad.validate().unwrap_err(),
        CrossRunnerSummaryError::ByteParityPendingNonSemanticMismatch {
            variant: 10,
            computed: 3,
        }
    ));
}

#[test]
fn validate_rejects_pending_unclassified_disagreement() {
    let bad = CrossRunnerSummary {
        convergence: Convergence::Yes,
        byte_parity: ByteParity::Pending {
            non_semantic_bytes: 3,
            unclassified_bytes: 10,
        },
        per_class_bytes: BTreeMap::from([
            (DivergenceClass::ElfHeader, 3),
            (DivergenceClass::Unclassified, 2),
        ]),
        unclassified_bytes: 2,
        unclassified_runs: vec![UnclassifiedRun {
            region_name: "r".to_string(),
            offset: 0,
            length: 2,
        }],
        lowest_offset_class: None,
    };
    assert!(matches!(
        bad.validate().unwrap_err(),
        CrossRunnerSummaryError::ByteParityPendingUnclassifiedMismatch {
            variant: 10,
            field: 2,
        }
    ));
}

#[test]
fn deserialize_routes_through_validate_and_rejects_invalid_pair() {
    let bad = serde_json::json!({
        "convergence": { "kind": "yes" },
        "byte_parity": {
            "kind": "diverge",
            "reason": { "kind": "outcome_mismatch", "cellgov": "Fault", "rpcs3": "Completed" }
        },
        "per_class_bytes": {},
        "unclassified_bytes": 0,
        "unclassified_runs": [],
        "lowest_offset_class": null
    });
    let parsed: Result<CrossRunnerSummary, _> = serde_json::from_value(bad);
    assert!(parsed.is_err());
}

#[test]
fn divergent_summary_json_round_trips() {
    let s = diverged(outcome_mismatch_reason());
    let json = serde_json::to_string_pretty(&s).unwrap();
    let parsed: CrossRunnerSummary = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, s);
}

#[test]
fn region_identity_at_index_0_wins_over_length_at_index_1() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![
            region("code", 0x10000, vec![0u8; 4]),
            region("data", 0x80000, vec![0u8; 4]),
        ],
        "cellgov",
        Some(1),
    );
    let b = obs(
        ObservedOutcome::Completed,
        vec![
            region("other", 0x10000, vec![0u8; 4]),
            region("data", 0x80000, vec![0u8; 8]),
        ],
        "rpcs3",
        Some(1),
    );
    let s = summarize(&compare_observations(&a, &b), &[]);
    assert!(matches!(
        s.convergence,
        Convergence::No {
            reason: ConvergenceFailure::RegionIdentityMismatch { index: 0, .. }
        }
    ));
}

#[test]
fn region_length_mismatch_wins_over_same_runner_step_mismatch() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0u8; 4])],
        "cellgov",
        Some(100),
    );
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0u8; 8])],
        "cellgov",
        Some(200),
    );
    let s = summarize(&compare_observations(&a, &b), &[]);
    assert!(matches!(
        s.convergence,
        Convergence::No {
            reason: ConvergenceFailure::RegionLengthMismatch { .. }
        }
    ));
}

#[test]
fn same_runner_step_mismatch_excludes_cross_runner_step_arm() {
    let a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(100));
    let b = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(200));
    let s = summarize(&compare_observations(&a, &b), &[]);
    assert!(matches!(
        s.convergence,
        Convergence::No {
            reason: ConvergenceFailure::SameRunnerStepMismatch { .. }
        }
    ));
}

#[test]
fn summary_json_round_trips() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0u8; 0x40])],
        "cellgov",
        Some(1),
    );
    let mut b_data = vec![0u8; 0x40];
    b_data[0x35] = 0x40;
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, b_data)],
        "rpcs3",
        Some(1),
    );
    let s = summarize(&compare_observations(&a, &b), &[DivergenceClass::ElfHeader]);
    let json = serde_json::to_string_pretty(&s).unwrap();
    let parsed: CrossRunnerSummary = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, s);
}

#[test]
fn byte_divergence_constructs_for_summary_input() {
    let _b = ByteDivergence {
        offset: 0,
        length: 1,
        a_byte: 0,
        b_byte: 1,
    };
}
