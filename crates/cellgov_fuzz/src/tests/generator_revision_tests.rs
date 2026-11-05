//! A replay record names the generator revision of its target and
//! strategy, and the engine refuses a revision it no longer runs.

use super::*;

use std::path::Path;

use crate::artifact::{
    ArtifactCheckSelection, ArtifactError, ArtifactExecutionPolicy, ArtifactReductionRequest,
    ArtifactReference, FuzzFindingArtifact,
};
use crate::seeded::{seed, SeededDefect};
use crate::{FuzzConfig, FuzzTarget};

const TARGETS: [FuzzTarget; 4] = [
    FuzzTarget::PpuInstruction,
    FuzzTarget::PpuSequence,
    FuzzTarget::SpuInstruction,
    FuzzTarget::SpuSequence,
];

/// The revision each mapping runs under; see [`generator_revision`] for why.
///
/// The match has no wildcard arm, so a new target or strategy fails to
/// compile here.
fn expected_revision(target: FuzzTarget, strategy: GenerationStrategy) -> GeneratorRevision {
    use FuzzTarget::{PpuInstruction, PpuSequence, SpuInstruction, SpuSequence};
    use GenerationStrategy::{RawWords, Structured};
    GeneratorRevision(match (target, strategy) {
        (PpuInstruction | PpuSequence, Structured) => 2,
        (PpuInstruction | PpuSequence, RawWords) => 1,
        (SpuInstruction | SpuSequence, Structured | RawWords) => 1,
    })
}

#[test]
fn a_record_with_no_revision_replays_on_no_mapping() {
    for target in TARGETS {
        for strategy in [GenerationStrategy::Structured, GenerationStrategy::RawWords] {
            let supported = expected_revision(target, strategy);
            assert_eq!(
                generator_revision(target, strategy),
                supported,
                "{target:?} {strategy:?}"
            );
            let unrevised: ReplayCoordinates = serde_json::from_value(serde_json::json!({
                "campaign_version": CAMPAIGN_VERSION.0,
                "target": target,
                "strategy": strategy,
                "seed": 1,
                "case_index": 0,
                "sequence_words": 32,
            }))
            .expect("parses");
            assert_eq!(unrevised.generator_revision, GeneratorRevision(0));
            assert_eq!(
                unrevised.validate(),
                Err(ReplayVersionError::GeneratorRevision {
                    target,
                    strategy,
                    found: GeneratorRevision(0),
                    supported,
                }),
                "{target:?} {strategy:?}"
            );
            let current = ReplayCoordinates::new(target, strategy, 1, 0, 32);
            assert_eq!(current.generator_revision, supported);
            assert_eq!(current.validate(), Ok(()), "{target:?} {strategy:?}");
        }
    }
}

#[test]
fn a_record_names_its_revision_and_one_with_no_revision_serializes_without_it() {
    let replay = ReplayCoordinates::new(
        FuzzTarget::PpuInstruction,
        GenerationStrategy::RawWords,
        1,
        5,
        32,
    );
    assert_eq!(
        serde_json::to_string(&replay).expect("serializes"),
        concat!(
            r#"{"campaign_version":4,"generator_revision":1,"target":"PpuInstruction","#,
            r#""strategy":"raw_words","seed":1,"case_index":5,"sequence_words":32}"#
        )
    );
    let unrevised = ReplayCoordinates {
        generator_revision: GeneratorRevision(0),
        ..replay
    };
    assert_eq!(
        serde_json::to_string(&unrevised).expect("serializes"),
        concat!(
            r#"{"campaign_version":4,"target":"PpuInstruction","strategy":"raw_words","#,
            r#""seed":1,"case_index":5,"sequence_words":32}"#
        )
    );
}

#[test]
fn a_ppu_raw_word_artifact_recorded_before_the_control_registers_were_drawn_is_refused() {
    let json = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("regressions/ppu-raw-sthu-ra-zero-debug-invariant.json"),
    )
    .expect("the regression is tracked");
    FuzzFindingArtifact::parse_json(&json).expect("the current record parses");
    let field = "\"generator_revision\": 1,";
    assert!(json.contains(field), "{json}");
    let before = json.replacen(field, "", 1);
    assert!(matches!(
        FuzzFindingArtifact::parse_json(&before),
        Err(ArtifactError::ReplayVersion(
            ReplayVersionError::GeneratorRevision {
                target: FuzzTarget::PpuInstruction,
                strategy: GenerationStrategy::RawWords,
                found: GeneratorRevision(0),
                supported: GeneratorRevision(1),
            }
        ))
    ));
}

#[test]
fn an_spu_sequence_artifact_recorded_before_the_revision_is_refused() {
    let config = FuzzConfig {
        seed: 11,
        schedule: crate::CampaignSchedule {
            cases: crate::CaseRange {
                first: 0,
                count: 16,
            },
            ..crate::CampaignSchedule::default()
        },
        max_findings: 4,
        sequence_words: 6,
        ..FuzzConfig::default()
    };
    let artifact = {
        let _guard = seed(SeededDefect::IllegalFootprint);
        let run = FuzzTarget::SpuSequence.run(config);
        let finding = run.report.findings.first().expect("a seeded finding");
        FuzzFindingArtifact::from_finding(
            config,
            ArtifactExecutionPolicy {
                workers: 1,
                deadline_ms: None,
                progress: false,
                check: ArtifactCheckSelection::All,
                reduction: ArtifactReductionRequest::None,
            },
            &run.report,
            finding,
            ArtifactReference::Local,
            Path::new("evidence/revision.json"),
        )
        .expect("artifact")
    };
    let json = serde_json::to_string(&artifact).expect("serializes");
    FuzzFindingArtifact::parse_json(&json).expect("the current record parses");
    let field = r#""generator_revision":1,"#;
    assert!(json.contains(field), "{json}");
    let before = json.replace(field, "");
    let refusal = FuzzFindingArtifact::parse_json(&before).expect_err("a stale revision");
    assert_eq!(
        refusal.to_string(),
        "finding artifact replay version: replay generator revision 0 of SpuSequence Structured \
         does not match supported revision 1"
    );
    assert!(matches!(
        Err::<(), _>(refusal),
        Err(ArtifactError::ReplayVersion(
            ReplayVersionError::GeneratorRevision {
                target: FuzzTarget::SpuSequence,
                strategy: GenerationStrategy::Structured,
                found: GeneratorRevision(0),
                supported: GeneratorRevision(1),
            }
        ))
    ));
}
