use super::*;
use std::path::Path;

use crate::report::{CheckIdentity, DivergenceClass, RunOutcome, SemanticFingerprint};
use crate::report::{Finding, FindingKind, FuzzReport, FuzzRun, ReductionOutcome};
use crate::{
    CampaignSchedule, CampaignShard, CaseRange, FuzzConfig, FuzzTarget, ReplayCoordinates,
    TargetPanicPayload,
};
use crate::{
    CampaignVersion, CancellationBoundary, CaseEligibility, CrossReferenceAsymmetry, FuzzError,
    GenerationStrategy, InvariantError, OperandAliasClass, SemanticObservation,
    StateTransitionClass, CAMPAIGN_VERSION,
};

fn sample() -> (FuzzFindingArtifact, FuzzRun) {
    let config = FuzzConfig {
        campaign_version: CAMPAIGN_VERSION,
        seed: 7,
        strategy: GenerationStrategy::Structured,
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 12,
                count: 1,
            },
            shard: CampaignShard::ALL,
            cancellation: None,
        },
        max_findings: 4,
        ..FuzzConfig::default()
    };
    let mut report = FuzzReport::new(
        FuzzTarget::PpuInstruction,
        7,
        config.strategy,
        config.retention,
        4,
        config.sequence_words,
    );
    report.considered().expect("one case");
    report.reached_many(1, []).expect("one decoded word");
    report
        .finding(Finding {
            fingerprint: SemanticFingerprint {
                target: FuzzTarget::PpuInstruction,
                instruction_kind: None,
                check: CheckIdentity::LegalOutcome,
                divergence: DivergenceClass::Outcome,
                outcome: None,
                effect: None,
            },
            kind: FindingKind::IllegalOutcome,
            replay: ReplayCoordinates {
                campaign_version: CampaignVersion(4),
                target: FuzzTarget::PpuInstruction,
                strategy: GenerationStrategy::Structured,
                seed: 7,
                case_index: 12,
                sequence_words: config.sequence_words,
            },
            original_words: vec![0x3860_0007],
            observation: None,
            reduction: ReductionOutcome::NotAttempted,
            panic_payload: None,
        })
        .expect("typed finding");
    report
        .observe_case(
            12,
            SemanticObservation {
                first_instruction_kind: None,
                instruction_kinds: Default::default(),
                operands: OperandAliasClass::Ordinary,
                eligibility: CaseEligibility::Eligible,
                outcome: None,
                state_transition: StateTransitionClass::ArchitecturalState,
                effects: Default::default(),
                boundaries: Default::default(),
                sequence_depth: 1,
                asymmetry: CrossReferenceAsymmetry::Outcome,
            },
        )
        .expect("observation retained on finding");
    let artifact = FuzzFindingArtifact::from_finding(
        config,
        ArtifactExecutionPolicy {
            workers: 2,
            deadline_ms: Some(500),
            progress: true,
            check: ArtifactCheckSelection::All,
            reduction: ArtifactReductionRequest::None,
        },
        &report,
        &report.findings[0],
        ArtifactReference::Local,
        Path::new("evidence/finding.json"),
    )
    .expect("artifact");
    (
        artifact,
        FuzzRun {
            outcome: RunOutcome::SemanticFinding,
            report,
        },
    )
}

#[test]
fn artifact_round_trip_replays_the_original_fingerprint_and_observation() {
    let (artifact, run) = sample();
    let json = serde_json::to_string_pretty(&artifact).expect("serializes");
    let parsed = FuzzFindingArtifact::parse_json(&json).expect("valid artifact");
    assert_eq!(parsed, artifact);
    assert_eq!(parsed.original.words, vec![0x3860_0007]);
    assert_eq!(
        parsed
            .observation
            .as_ref()
            .map(|observation| observation.sequence_depth),
        Some(1)
    );
    assert_eq!(
        parsed.replay_command,
        [
            "cellgov",
            "dev",
            "fuzz",
            "replay",
            "--artifact",
            "evidence/finding.json"
        ]
    );
    let replayed = parsed
        .replay_with(|config| {
            assert_eq!(
                config.schedule.cases,
                CaseRange {
                    first: 12,
                    count: 1
                }
            );
            assert_eq!(config.schedule.shard, CampaignShard::ALL);
            run.clone()
        })
        .expect("same fingerprint");
    assert_eq!(
        ArtifactFingerprint::from(&replayed.fingerprint),
        parsed.fingerprint
    );
}

#[test]
fn changed_result_and_version_refuse_before_claiming_reproduction() {
    let (artifact, mut run) = sample();
    run.report.findings[0].fingerprint.divergence = DivergenceClass::ControlFlow;
    assert!(matches!(
        artifact.replay_with(|_| run),
        Err(ArtifactReplayError::NotReproduced { case_index: 12 })
    ));
    let mut changed = artifact.clone();
    changed.schema_version += 1;
    assert!(matches!(
        changed.replay_with(|_| panic!("must not execute incompatible schema")),
        Err(ArtifactReplayError::Artifact(ArtifactError::Version { .. }))
    ));
    changed = artifact;
    changed.original.replay.campaign_version.0 += 1;
    assert!(matches!(
        changed.replay_with(|_| panic!("must not execute changed generator")),
        Err(ArtifactReplayError::Artifact(ArtifactError::ReplayVersion(
            _
        )))
    ));
}

#[test]
fn a_changed_panic_payload_is_not_a_reproduction() {
    let (artifact, mut run) = sample();
    run.report.findings[0].panic_payload =
        Some(TargetPanicPayload::StaticStr("index out of bounds".into()));
    assert!(matches!(
        artifact.replay_with(|_| run),
        Err(ArtifactReplayError::NotReproduced { case_index: 12 })
    ));
}

#[test]
fn a_case_outside_the_shard_or_cancellation_refuses_before_running() {
    let (artifact, _) = sample();
    let mut sharded = artifact.clone();
    sharded.campaign.schedule.cases.count = 2;
    sharded.campaign.schedule.shard = CampaignShard { index: 1, count: 2 };
    assert!(matches!(
        sharded.replay_with(|_| panic!("must not execute a case the shard never scheduled")),
        Err(ArtifactReplayError::Artifact(ArtifactError::Invalid(_)))
    ));
    let mut cancelled = artifact.clone();
    cancelled.campaign.schedule.cancellation = Some(CancellationBoundary(0));
    assert!(matches!(
        cancelled.replay_with(|_| panic!("must not execute a cancelled case")),
        Err(ArtifactReplayError::Artifact(ArtifactError::Invalid(_)))
    ));
    let mut preceding = artifact;
    preceding.campaign.schedule.cases.first = 13;
    assert!(matches!(
        preceding.replay_with(|_| panic!("must not execute a case before the range")),
        Err(ArtifactReplayError::Artifact(ArtifactError::Invalid(_)))
    ));
}

#[test]
fn decoded_words_are_bounded_per_case_by_the_engine() {
    let (artifact, _) = sample();
    let mut instruction = artifact.clone();
    instruction.coverage.decoded = 2;
    assert!(matches!(
        instruction.validate(),
        Err(ArtifactError::Invalid(
            "finding or coverage context is inconsistent"
        ))
    ));
    // A sequence engine decodes up to `sequence_words` words per case.
    let mut sequence = artifact;
    sequence.original.replay.target = FuzzTarget::PpuSequence;
    sequence.fingerprint.target = FuzzTarget::PpuSequence;
    let per_case = u64::from(sequence.campaign.sequence_words);
    assert!(per_case > 1);
    sequence.coverage.decoded = per_case;
    assert_eq!(
        sequence.validate().map_err(|error| error.to_string()),
        Ok(())
    );
    sequence.coverage.decoded = per_case + 1;
    assert!(matches!(
        sequence.validate(),
        Err(ArtifactError::Invalid(
            "finding or coverage context is inconsistent"
        ))
    ));
}

#[test]
fn optional_observation_and_failed_reduction_keep_original_words() {
    let (mut artifact, _) = sample();
    artifact.reduction = ArtifactReduction::Failed {
        reason: "seeded reduction refusal".into(),
    };
    let mut json = serde_json::to_value(&artifact).expect("serializes");
    json.as_object_mut().expect("object").remove("observation");
    let parsed = FuzzFindingArtifact::parse_json(&json.to_string()).expect("optional observation");
    assert!(parsed.observation.is_none());
    assert_eq!(parsed.original.words, vec![0x3860_0007]);
    assert!(matches!(parsed.reduction, ArtifactReduction::Failed { .. }));
}

#[test]
fn a_reduced_case_must_be_a_smaller_nonempty_case() {
    let (artifact, _) = sample();
    let mut irreducible = artifact.clone();
    irreducible.reduction = ArtifactReduction::Irreducible;
    let json = serde_json::to_string(&irreducible).expect("serializes");
    assert_eq!(
        FuzzFindingArtifact::parse_json(&json).expect("irreducible round trip"),
        irreducible
    );
    for words in [Vec::new(), vec![0x3860_0007], vec![0x3860_0007, 0]] {
        let mut larger = artifact.clone();
        larger.reduction = ArtifactReduction::Reduced { words };
        assert!(matches!(
            larger.validate(),
            Err(ArtifactError::Invalid(
                "reduction is not a smaller nonempty case"
            ))
        ));
    }
    let mut smaller = artifact;
    smaller.reduction = ArtifactReduction::Reduced {
        words: vec![0x3860_0006],
    };
    smaller.validate().expect("one cleared bit is smaller");
}

#[test]
fn reduced_replay_requires_a_reduced_case_that_still_reproduces() {
    let (artifact, _) = sample();
    assert!(matches!(
        artifact.replay_reduced(),
        Err(ArtifactReplayError::NoReducedCase)
    ));
    let mut reduced = artifact;
    reduced.reduction = ArtifactReduction::Reduced {
        words: vec![0x3860_0006],
    };
    assert!(matches!(
        reduced.replay_reduced(),
        Err(ArtifactReplayError::NotReproduced { case_index: 12 })
    ));
}

#[test]
fn a_reduced_replay_names_an_engine_failure_instead_of_a_missing_finding() {
    let (artifact, run) = sample();
    let mut reduced = artifact;
    reduced.reduction = ArtifactReduction::Reduced {
        words: vec![0x3860_0006],
    };
    let failed = FuzzRun::failed(
        run.report,
        FuzzError::from(InvariantError::EmptyGeneratedSequence),
    );
    assert!(matches!(
        reduced.reduced_finding(failed.clone()),
        Err(ArtifactReplayError::HarnessFailure {
            source: FuzzError::Invariant(InvariantError::EmptyGeneratedSequence)
        })
    ));
    assert!(matches!(
        reduced.replay_with(|_| failed),
        Err(ArtifactReplayError::HarnessFailure {
            source: FuzzError::Invariant(InvariantError::EmptyGeneratedSequence)
        })
    ));
}

#[test]
fn an_independent_vector_retains_versioned_provenance() {
    let fixture = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ppu_reference/li_r3_7_v1.json"
    ));
    let reference = ArtifactReference::ppu(fixture).expect("validated official vector");
    let (mut artifact, run) = sample();
    artifact.reference = reference;
    let parsed = FuzzFindingArtifact::parse_json(&serde_json::to_string(&artifact).expect("JSON"))
        .expect("versioned reference");
    assert!(matches!(parsed.reference, ArtifactReference::Ppu(_)));
    let replayed = parsed
        .replay_with(|_| run)
        .expect("a matching vector does not block replay");
    assert_eq!(
        ArtifactFingerprint::from(&replayed.fingerprint),
        parsed.fingerprint
    );
}

#[test]
fn artifact_key_order_is_pinned() {
    let (artifact, _) = sample();
    assert_eq!(
        serde_json::to_string(&artifact).expect("serializes"),
        concat!(
            r#"{"schema_version":2,"#,
            r#""campaign":{"campaign_version":4,"seed":7,"strategy":"structured","#,
            r#""schedule":{"cases":{"first":12,"count":1},"shard":{"index":0,"count":1},"cancellation":null},"#,
            r#""retention":{"capacity":256,"per_kind_capacity":8,"novelty_weight":8,"rarity_weight":4,"asymmetry_weight":16,"policy":"balanced"},"#,
            r#""max_findings":4,"sequence_words":32},"#,
            r#""execution":{"workers":2,"deadline_ms":500,"progress":true,"check":"all","reduction":{"kind":"none"}},"#,
            r#""original":{"replay":{"campaign_version":4,"target":"PpuInstruction","strategy":"structured","seed":7,"case_index":12,"sequence_words":32},"#,
            r#""words":[945815559],"state_source":"versioned_generator"},"#,
            r#""finding_kind":"IllegalOutcome","#,
            r#""fingerprint":{"target":"PpuInstruction","instruction_kind":null,"check":"LegalOutcome","divergence":"Outcome","outcome":null,"effect":null},"#,
            r#""reference":{"kind":"local"},"#,
            r#""observation":{"first_instruction_kind":null,"instruction_kinds":[],"operands":"Ordinary","eligibility":"Eligible","outcome":null,"state_transition":"ArchitecturalState","effects":[],"boundaries":[],"sequence_depth":1,"asymmetry":"Outcome"},"#,
            r#""coverage":{"cases":1,"decoded":1,"eligible":0,"unsupported":0,"undefined":0,"finding_counts":{"IllegalOutcome":1}},"#,
            r#""reduction":{"kind":"not_attempted"},"#,
            r#""panic_payload":null,"#,
            r#""replay_command":["cellgov","dev","fuzz","replay","--artifact","evidence/finding.json"]}"#,
        )
    );
}

/// The path is portable text: trailing separators of either kind are
/// trimmed and one forward slash joins the file. The comparison reads
/// the text, since a Windows path compares `\` and `/` as one separator.
#[test]
fn an_artifact_path_joins_the_directory_with_one_forward_slash() {
    for dir in ["out", "out/", "out\\", "out//"] {
        assert_eq!(
            artifact_path(dir, "PpuSequence-Structured-7", 12, 3).to_str(),
            Some("out/PpuSequence-Structured-7-12-3.json"),
            "{dir}"
        );
    }
}
