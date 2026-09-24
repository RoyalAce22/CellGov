use super::*;

use crate::artifact::{
    ArtifactCheckSelection, ArtifactExecutionPolicy, ArtifactReductionRequest, ArtifactReference,
};
use crate::reduce::{reduce_finding, ReductionPolicy, ReductionRequest};
use crate::seeded::{seed, SeededDefect};
use crate::{
    ppu, CampaignSchedule, CampaignShard, CaseRange, FindingKind, FuzzConfig, ReductionOutcome,
};

const SEEDED_CASES: u64 = 8;

fn config() -> FuzzConfig {
    FuzzConfig {
        seed: 100,
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 0,
                count: SEEDED_CASES,
            },
            shard: CampaignShard::ALL,
            cancellation: None,
        },
        max_findings: 8,
        ..FuzzConfig::default()
    }
}

fn policy() -> ArtifactExecutionPolicy {
    ArtifactExecutionPolicy {
        workers: 1,
        deadline_ms: None,
        progress: false,
        check: ArtifactCheckSelection::All,
        reduction: ArtifactReductionRequest::OnFinding {
            policy: ReductionPolicy::Deterministic,
            budget: 24,
        },
    }
}

/// The seeded illegal-outcome finding of the first case that carries one,
/// reduced when `minimized`.
fn seeded_artifact(path: &Path, minimized: bool) -> FuzzFindingArtifact {
    let _guard = seed(SeededDefect::IllegalOutcome);
    let config = config();
    let run = ppu::run_instructions(config);
    let mut finding = run
        .report
        .findings
        .first()
        .expect("the seeded defect yields a finding")
        .clone();
    assert_eq!(finding.kind, FindingKind::IllegalOutcome);
    if minimized {
        finding.reduction = reduce_finding(
            config,
            &finding,
            ReductionRequest {
                policy: ReductionPolicy::Deterministic,
                budget: 24,
            },
        )
        .expect("the seeded finding reduces")
        .into_outcome();
        assert!(matches!(
            finding.reduction,
            ReductionOutcome::Reduced(_) | ReductionOutcome::Irreducible
        ));
    }
    FuzzFindingArtifact::from_finding(
        config,
        policy(),
        &run.report,
        &finding,
        ArtifactReference::Local,
        path,
    )
    .expect("artifact")
}

fn write_artifact(dir: &Path, name: &str, minimized: bool) -> FuzzFindingArtifact {
    let path = dir.join(format!("{name}.json"));
    let artifact = seeded_artifact(&path, minimized);
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&artifact).expect("encodes"),
    )
    .expect("writes the artifact");
    artifact
}

fn entry(name: &str, status: RegressionStatus, profile: RegressionProfile) -> RegressionEntry {
    RegressionEntry {
        name: name.to_owned(),
        status,
        profile,
        summary: "seeded illegal outcome".to_owned(),
    }
}

fn write_manifest(dir: &Path, regressions: Vec<RegressionEntry>) {
    write_manifest_at(
        dir,
        &RegressionManifest {
            schema_version: REGRESSION_MANIFEST_VERSION,
            regressions,
        },
    );
}

fn write_manifest_at(dir: &Path, manifest: &RegressionManifest) {
    std::fs::write(
        dir.join(MANIFEST_FILE),
        serde_json::to_vec_pretty(manifest).expect("encodes"),
    )
    .expect("writes the manifest");
}

fn verify_all(dir: &Path) -> Result<(), RegressionError> {
    for regression in load(dir)? {
        regression.verify()?;
    }
    Ok(())
}

#[test]
fn an_open_regression_must_reproduce_and_a_fixed_one_must_not() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("regression_open_fixed");
    let dir: &Path = &scratch;
    let artifact = write_artifact(dir, "seeded-illegal-outcome", true);
    write_manifest(
        dir,
        vec![entry(
            "seeded-illegal-outcome",
            RegressionStatus::Open,
            RegressionProfile::Both,
        )],
    );
    let loaded = load(dir).expect("loads");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].artifact, artifact);
    assert_eq!(loaded[0].path, dir.join("seeded-illegal-outcome.json"));
    assert!(loaded[0].expected_to_reproduce());
    {
        let _guard = seed(SeededDefect::IllegalOutcome);
        verify_all(dir).expect("the open regression reproduces with its defect");
    }
    assert!(
        matches!(
            verify_all(dir),
            Err(RegressionError::Stale { name }) if name == "seeded-illegal-outcome"
        ),
        "an open regression whose defect is gone is stale"
    );

    write_manifest(
        dir,
        vec![entry(
            "seeded-illegal-outcome",
            RegressionStatus::Fixed,
            RegressionProfile::Both,
        )],
    );
    assert!(!load(dir).expect("loads")[0].expected_to_reproduce());
    verify_all(dir).expect("the fixed regression stays fixed without its defect");
    let _guard = seed(SeededDefect::IllegalOutcome);
    assert!(
        matches!(
            verify_all(dir),
            Err(RegressionError::Regressed { name }) if name == "seeded-illegal-outcome"
        ),
        "a fixed regression whose defect returns has regressed"
    );
}

#[test]
fn a_profile_binds_the_expectation_to_the_running_build() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("regression_profile");
    let dir: &Path = &scratch;
    write_artifact(dir, "seeded", true);
    let (this_build, other_build) = if cfg!(debug_assertions) {
        (RegressionProfile::Debug, RegressionProfile::Release)
    } else {
        (RegressionProfile::Release, RegressionProfile::Debug)
    };
    assert!(this_build.matches_build());
    assert!(!other_build.matches_build());
    assert!(RegressionProfile::Both.matches_build());

    write_manifest(
        dir,
        vec![entry("seeded", RegressionStatus::Open, this_build)],
    );
    assert!(load(dir).expect("loads")[0].expected_to_reproduce());
    {
        let _guard = seed(SeededDefect::IllegalOutcome);
        verify_all(dir).expect("an open regression of this build's profile reproduces");
    }
    assert!(matches!(
        verify_all(dir),
        Err(RegressionError::Stale { .. })
    ));

    write_manifest(
        dir,
        vec![entry("seeded", RegressionStatus::Open, other_build)],
    );
    assert!(!load(dir).expect("loads")[0].expected_to_reproduce());
    verify_all(dir).expect("an open regression of the other profile must not reproduce here");
    let _guard = seed(SeededDefect::IllegalOutcome);
    assert!(matches!(
        verify_all(dir),
        Err(RegressionError::Regressed { .. })
    ));
}

#[test]
fn a_directory_and_its_manifest_must_agree() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("regression_agreement");
    let dir: &Path = &scratch;
    let open = |name: &str| entry(name, RegressionStatus::Open, RegressionProfile::Both);

    write_manifest(dir, vec![open("absent")]);
    assert!(matches!(
        load(dir),
        Err(RegressionError::Read { path, .. }) if path == dir.join("absent.json")
    ));

    write_artifact(dir, "unreduced", false);
    write_manifest(dir, vec![open("unreduced")]);
    assert!(matches!(
        load(dir),
        Err(RegressionError::NotMinimized { name }) if name == "unreduced"
    ));
    std::fs::remove_file(dir.join("unreduced.json")).expect("removes");

    write_artifact(dir, "seeded", true);
    write_manifest(dir, vec![open("seeded"), open("seeded")]);
    assert!(matches!(
        load(dir),
        Err(RegressionError::DuplicateName { name }) if name == "seeded"
    ));

    write_manifest(dir, vec![open("Seeded Finding")]);
    assert!(matches!(
        load(dir),
        Err(RegressionError::InvalidName { name }) if name == "Seeded Finding"
    ));

    let mut blank = open("seeded");
    blank.summary = "  ".to_owned();
    write_manifest(dir, vec![blank]);
    assert!(matches!(
        load(dir),
        Err(RegressionError::EmptySummary { name }) if name == "seeded"
    ));

    write_artifact(dir, "seeded-again", true);
    write_manifest(dir, vec![open("seeded"), open("seeded-again")]);
    assert!(matches!(
        load(dir),
        Err(RegressionError::DuplicateFinding { name, other })
            if name == "seeded-again" && other == "seeded"
    ));
    std::fs::remove_file(dir.join("seeded-again.json")).expect("removes");

    std::fs::write(dir.join("notes.json"), b"{}").expect("writes a stray file");
    write_manifest(dir, vec![open("seeded")]);
    assert!(matches!(
        load(dir),
        Err(RegressionError::Stray { path }) if path == dir.join("notes.json")
    ));
    std::fs::remove_file(dir.join("notes.json")).expect("removes");
    std::fs::write(dir.join("README.md"), b"notes\n").expect("writes a readme");
    assert_eq!(load(dir).expect("a readme is not a stray file").len(), 1);

    write_manifest_at(
        dir,
        &RegressionManifest {
            schema_version: REGRESSION_MANIFEST_VERSION + 1,
            regressions: vec![open("seeded")],
        },
    );
    assert!(matches!(
        load(dir),
        Err(RegressionError::Version {
            found: 2,
            supported: 1
        })
    ));

    std::fs::write(dir.join(MANIFEST_FILE), b"[]").expect("writes a non-manifest");
    assert!(matches!(load(dir), Err(RegressionError::Manifest { .. })));

    std::fs::remove_file(dir.join(MANIFEST_FILE)).expect("removes");
    assert!(matches!(
        load(dir),
        Err(RegressionError::Read { path, .. }) if path == dir.join(MANIFEST_FILE)
    ));
}

#[test]
fn promotion_matches_by_kind_and_fingerprint_in_the_running_build() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("regression_promotion");
    let dir: &Path = &scratch;
    write_artifact(dir, "seeded", true);
    write_manifest(
        dir,
        vec![entry(
            "seeded",
            RegressionStatus::Open,
            RegressionProfile::Both,
        )],
    );
    let regressions = load(dir).expect("loads");
    let seeded = {
        let _guard = seed(SeededDefect::IllegalOutcome);
        ppu::run_instructions(config()).report.findings
    };
    let finding = seeded.first().expect("the seeded defect yields a finding");
    assert_eq!(
        promoted(&regressions, finding).map(|regression| regression.entry.name.as_str()),
        Some("seeded"),
        "the promoted finding covers its own class"
    );
    // Every seeded finding of another instruction kind is another class.
    let mut other_classes = 0;
    for candidate in seeded
        .iter()
        .filter(|candidate| !regressions[0].covers(candidate))
    {
        other_classes += 1;
        assert!(promoted(&regressions, candidate).is_none());
    }
    assert!(other_classes > 0, "the seeded budget spans several kinds");
    let mut other_kind = finding.clone();
    other_kind.kind = FindingKind::IllegalEffect;
    assert!(promoted(&regressions, &other_kind).is_none());
    let mut other_class = finding.clone();
    other_class.fingerprint.instruction_kind = None;
    assert!(promoted(&regressions, &other_class).is_none());

    write_manifest(
        dir,
        vec![entry(
            "seeded",
            RegressionStatus::Fixed,
            RegressionProfile::Both,
        )],
    );
    let fixed = load(dir).expect("loads");
    assert!(
        promoted(&fixed, &seeded[0]).is_none(),
        "a fixed regression excuses no finding"
    );
}

#[test]
fn promotion_stores_the_artifact_under_its_name_and_lists_it_open() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("regression_promote");
    let dir: &Path = &scratch;
    write_manifest(dir, Vec::new());
    let artifact = seeded_artifact(&dir.join("elsewhere.json"), true);
    let promoted_entry = promote(
        dir,
        "seeded",
        RegressionProfile::Both,
        " seeded illegal outcome ",
        &artifact,
    )
    .expect("promotes");
    assert_eq!(promoted_entry.entry.name, "seeded");
    assert_eq!(promoted_entry.entry.status, RegressionStatus::Open);
    assert_eq!(promoted_entry.entry.summary, "seeded illegal outcome");
    assert_eq!(promoted_entry.path, dir.join("seeded.json"));
    assert_eq!(
        promoted_entry.artifact.replay_command[5],
        format!(
            "{}/seeded.json",
            dir.to_str().expect("utf-8").replace('\\', "/")
        ),
        "the stored replay path is spelled with forward slashes on every host"
    );
    assert!(!promoted_entry.artifact.replay_command[5].contains('\\'));
    let stored = std::fs::read(dir.join("seeded.json")).expect("stored file reads");
    assert!(
        stored.ends_with(b"}\n") && !stored.ends_with(b"\n\n"),
        "a promoted regression ends with exactly one newline"
    );
    assert!(promoted_entry.artifact.describes_same_finding(&artifact));
    let loaded = load(dir).expect("loads");
    assert_eq!(loaded, vec![promoted_entry]);
    {
        let _guard = seed(SeededDefect::IllegalOutcome);
        verify_all(dir).expect("the promoted regression reproduces");
    }

    assert!(matches!(
        promote(dir, "seeded", RegressionProfile::Both, "again", &artifact),
        Err(RegressionError::DuplicateName { name }) if name == "seeded"
    ));
    assert!(matches!(
        promote(dir, "seeded-twice", RegressionProfile::Both, "again", &artifact),
        Err(RegressionError::DuplicateFinding { name, other })
            if name == "seeded-twice" && other == "seeded"
    ));
    assert!(matches!(
        promote(dir, "Seeded", RegressionProfile::Both, "again", &artifact),
        Err(RegressionError::InvalidName { .. })
    ));
    let unreduced = seeded_artifact(&dir.join("elsewhere.json"), false);
    let mut other_kind = unreduced.clone();
    other_kind.finding_kind = "IllegalEffect".to_owned();
    assert!(matches!(
        promote(dir, "unreduced", RegressionProfile::Both, "again", &other_kind),
        Err(RegressionError::NotMinimized { name }) if name == "unreduced"
    ));
    let mut other_class = artifact.clone();
    other_class.fingerprint.instruction_kind = None;
    assert!(matches!(
        promote(dir, "blank", RegressionProfile::Both, "  ", &other_class),
        Err(RegressionError::EmptySummary { name }) if name == "blank"
    ));
    assert_eq!(
        load(dir).expect("loads").len(),
        1,
        "a refused promotion writes nothing"
    );
}

#[test]
fn the_tracked_regressions_are_witnesses_in_this_build() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("regressions");
    let manifest: RegressionManifest = serde_json::from_str(
        &std::fs::read_to_string(dir.join(MANIFEST_FILE)).expect("the manifest is tracked"),
    )
    .expect("the manifest parses");
    let regressions = load(&dir).expect("the tracked regressions load");
    assert_eq!(regressions.len(), manifest.regressions.len());
    for regression in &regressions {
        assert!(
            !regression.entry.summary.trim().is_empty(),
            "{} has no summary",
            regression.entry.name
        );
        regression
            .verify()
            .unwrap_or_else(|error| panic!("{}: {error}", regression.path.display()));
    }
}
