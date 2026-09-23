use super::outcome::*;

use std::collections::BTreeMap;
use std::path::PathBuf;

use cellgov_fuzz::artifact::{
    ArtifactError, ArtifactFingerprint, ArtifactReduction, ArtifactReplayError,
};
use cellgov_fuzz::raw_decode::{RawDecodeStatus, RawDecoder};
use cellgov_fuzz::{FindingKind, FuzzError, FuzzTarget, InvariantError};

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::exit_codes;
use crate::cli::fuzz::FuzzCliError;
use crate::cli::parse::{
    CAMPAIGN_EXIT_CODES, RAW_EXIT_CODES, REPLAY_EXIT_CODES, SEMANTIC_EXIT_CODES,
};

fn fingerprint() -> ArtifactFingerprint {
    ArtifactFingerprint {
        target: FuzzTarget::PpuInstruction,
        instruction_kind: None,
        check: "LegalOutcome".into(),
        divergence: "Outcome".into(),
        outcome: None,
        effect: None,
    }
}

fn record(stored: bool, reduction: ArtifactReduction) -> ArtifactRecord {
    ArtifactRecord {
        path: PathBuf::from("out").join("finding.json"),
        campaign_version: 3,
        seed: 7,
        case_index: 12,
        finding_kind: FindingKind::IllegalOutcome,
        fingerprint: fingerprint(),
        reduction,
        stored,
    }
}

fn summary_with_findings(findings: u64) -> CampaignSummary {
    CampaignSummary {
        cases: 4,
        decoded: 4,
        eligible: 3,
        unsupported: 1,
        undefined: 0,
        finding_counts: if findings == 0 {
            BTreeMap::new()
        } else {
            BTreeMap::from([(FindingKind::IllegalOutcome, findings)])
        },
        artifacts: Vec::new(),
        reductions_failed: 0,
        cancelled: false,
    }
}

#[test]
fn campaign_outcomes_follow_a_fixed_precedence() {
    let clean = summary_with_findings(0);
    assert_eq!(
        CampaignOutcome::classify(&clean, false, false),
        CampaignOutcome::Clean
    );
    let mut cancelled = clean.clone();
    cancelled.cancelled = true;
    assert_eq!(
        CampaignOutcome::classify(&cancelled, false, false),
        CampaignOutcome::Cancelled
    );
    let mut ineligible = clean.clone();
    ineligible.eligible = 0;
    assert_eq!(
        CampaignOutcome::classify(&ineligible, false, false),
        CampaignOutcome::NoEligibleCases
    );
    ineligible.cancelled = true;
    assert_eq!(
        CampaignOutcome::classify(&ineligible, false, false),
        CampaignOutcome::Cancelled,
        "a cancelled range cannot claim every case ran"
    );
    let mut classified = clean.clone();
    classified.finding_counts = BTreeMap::from([
        (FindingKind::Unsupported, 3),
        (FindingKind::Undefined, u64::MAX),
    ]);
    assert_eq!(classified.findings(), 0);
    assert_eq!(
        CampaignOutcome::classify(&classified, false, false),
        CampaignOutcome::Clean,
        "an inapplicable case is a classification, not a finding"
    );
    classified
        .finding_counts
        .insert(FindingKind::TargetPanic, u64::MAX);
    classified
        .finding_counts
        .insert(FindingKind::IllegalEffect, 1);
    assert_eq!(classified.findings(), u64::MAX);
    let findings = summary_with_findings(2);
    assert_eq!(
        CampaignOutcome::classify(&findings, false, false),
        CampaignOutcome::Findings
    );
    let mut cut_short = findings.clone();
    cut_short.cancelled = true;
    assert_eq!(
        CampaignOutcome::classify(&cut_short, false, false),
        CampaignOutcome::Cancelled,
        "a cut range is incomplete whatever it found"
    );
    let mut reduced = findings.clone();
    reduced.reductions_failed = 1;
    assert_eq!(
        CampaignOutcome::classify(&reduced, false, false),
        CampaignOutcome::ReductionFailed
    );
    assert_eq!(
        CampaignOutcome::classify(&reduced, false, true),
        CampaignOutcome::HarnessFailure
    );
    assert_eq!(
        CampaignOutcome::classify(&reduced, true, true),
        CampaignOutcome::EvidenceNotStored
    );
}

#[test]
fn every_campaign_outcome_has_the_documented_status() {
    for (outcome, code) in [
        (CampaignOutcome::Clean, 0),
        (CampaignOutcome::Findings, 0),
        (CampaignOutcome::Cancelled, EXIT_CANCELLED),
        (CampaignOutcome::NoEligibleCases, EXIT_NO_ELIGIBLE_CASES),
        (CampaignOutcome::HarnessFailure, EXIT_HARNESS_FAILURE),
        (CampaignOutcome::EvidenceNotStored, EXIT_EVIDENCE_NOT_STORED),
        (CampaignOutcome::ReductionFailed, EXIT_REDUCTION_FAILED),
    ] {
        assert_eq!(
            outcome.exit_code(),
            CommandExitCode::new(code),
            "{outcome:?}"
        );
    }
}

#[test]
fn the_error_backed_outcomes_share_their_status_with_the_typed_error() {
    let harness = FuzzCliError::Harness(FuzzError::from(InvariantError::EmptyGeneratedSequence));
    assert_eq!(harness.exit_code(), EXIT_HARNESS_FAILURE);
    let ineligible = FuzzCliError::NoEligibleCases {
        cases: 1,
        decoded: 0,
        unsupported: 0,
        undefined: 0,
    };
    assert_eq!(ineligible.exit_code(), EXIT_NO_ELIGIBLE_CASES);
    let unstored = FuzzCliError::ArtifactCollision {
        path: PathBuf::from("finding.json"),
        artifact: Box::new(crate::cli::fuzz::tests::synthetic_finding_artifact()),
    };
    assert_eq!(unstored.exit_code(), EXIT_EVIDENCE_NOT_STORED);
    let not_reproduced =
        FuzzCliError::ArtifactReplay(cellgov_fuzz::artifact::ArtifactReplayError::NotReproduced {
            case_index: 3,
        });
    assert_eq!(not_reproduced.exit_code(), EXIT_NOT_REPRODUCED);
    let incompatible = FuzzCliError::Artifact(ArtifactError::Version {
        found: 1,
        supported: 2,
    });
    assert_eq!(incompatible.exit_code(), exit_codes::USAGE);
    let no_reduced_case = FuzzCliError::ArtifactReplay(ArtifactReplayError::NoReducedCase);
    assert_eq!(no_reduced_case.exit_code(), exit_codes::USAGE);
    assert_eq!(
        CommandError::from(harness).code().expect("status").value(),
        u8::try_from(EXIT_HARNESS_FAILURE).expect("small code")
    );
}

#[test]
fn every_particular_status_is_named_in_its_command_help() {
    for (help, codes) in [
        (
            CAMPAIGN_EXIT_CODES,
            vec![
                exit_codes::FAILED,
                EXIT_CANCELLED,
                EXIT_NO_ELIGIBLE_CASES,
                EXIT_HARNESS_FAILURE,
                EXIT_EVIDENCE_NOT_STORED,
                EXIT_REDUCTION_FAILED,
            ],
        ),
        (SEMANTIC_EXIT_CODES, vec![exit_codes::FAILED]),
        (RAW_EXIT_CODES, vec![exit_codes::FAILED, EXIT_CANCELLED]),
        (
            REPLAY_EXIT_CODES,
            vec![
                exit_codes::FAILED,
                EXIT_HARNESS_FAILURE,
                EXIT_NOT_REPRODUCED,
            ],
        ),
    ] {
        for code in codes {
            assert!(
                help.lines().any(|line| {
                    line.split_whitespace()
                        .next()
                        .is_some_and(|first| first == code.to_string())
                }),
                "help does not name {code}:\n{help}"
            );
        }
    }
}

#[test]
fn the_summary_groups_artifacts_by_finding_and_names_one_replay_each() {
    let mut summary = summary_with_findings(2);
    summary.artifacts = vec![
        record(true, ArtifactReduction::NotAttempted),
        record(
            false,
            ArtifactReduction::Reduced {
                words: vec![0x3860_0006],
            },
        ),
    ];
    summary.reductions_failed = 0;
    let text = render_campaign_summary(
        FuzzTarget::PpuInstruction,
        &summary,
        CampaignOutcome::Findings,
    );
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(
        lines[0],
        "fuzz: PpuInstruction cases=4 decoded=4 eligible=3 unsupported=1 undefined=0 findings=2 reductions_failed=0 cancelled=false outcome=Findings"
    );
    let replay = summary.artifacts[0].replay_command();
    assert_eq!(
        replay,
        format!(
            "cellgov dev fuzz replay --artifact {}",
            PathBuf::from("out").join("finding.json").display()
        )
    );
    assert_eq!(
        lines[1],
        format!("fuzz: findings=2 kind=IllegalOutcome check=LegalOutcome divergence=Outcome instructions=none reduced=1 irreducible=0 reduction_failed=0 replay: {replay}")
    );
    assert_eq!(
        lines[2],
        format!(
            "fuzz: artifacts=2 stored=1 not_stored=1 dir={}",
            PathBuf::from("out").display()
        )
    );
    assert_eq!(
        text,
        render_campaign_summary(
            FuzzTarget::PpuInstruction,
            &summary,
            CampaignOutcome::Findings
        )
    );
}

fn artifact_on(kind: FindingKind, check: &str, instruction: &str, case: u64) -> ArtifactRecord {
    let mut record = record(true, ArtifactReduction::NotAttempted);
    record.finding_kind = kind;
    record.fingerprint.check = check.into();
    record.fingerprint.instruction_kind = Some(instruction.into());
    record.case_index = case;
    record.path = PathBuf::from("out").join(format!("{case}.json"));
    record
}

#[test]
fn a_group_names_the_earliest_stored_artifact_as_its_replay() {
    let mut unstored = artifact_on(
        FindingKind::IllegalOutcome,
        "LegalOutcome",
        "Ppu(Ordinary(Stw))",
        1,
    );
    unstored.stored = false;
    let stored = artifact_on(
        FindingKind::IllegalOutcome,
        "LegalOutcome",
        "Ppu(Ordinary(Stw))",
        2,
    );
    let mut summary = summary_with_findings(2);
    summary.artifacts = vec![unstored.clone(), stored.clone()];
    let text = render_campaign_summary(
        FuzzTarget::PpuInstruction,
        &summary,
        CampaignOutcome::EvidenceNotStored,
    );
    let group = text.lines().nth(1).expect("group line");
    assert!(
        group.ends_with(&format!("replay: {}", stored.replay_command())),
        "{group}"
    );
    // With nothing stored, the earliest record still names the path
    // the write targeted.
    summary.artifacts = vec![unstored.clone()];
    let text = render_campaign_summary(
        FuzzTarget::PpuInstruction,
        &summary,
        CampaignOutcome::EvidenceNotStored,
    );
    let group = text.lines().nth(1).expect("group line");
    assert!(
        group.ends_with(&format!("replay: {}", unstored.replay_command())),
        "{group}"
    );
}

#[test]
fn finding_groups_sort_by_size_list_their_instructions_and_cap_the_list() {
    let mut summary = summary_with_findings(8);
    summary.artifacts = vec![
        artifact_on(
            FindingKind::IllegalOutcome,
            "LegalOutcome",
            "Ppu(Ordinary(Stw))",
            1,
        ),
        artifact_on(
            FindingKind::MetamorphicViolation,
            "PpuRecordCr6",
            "Ppu(Vx(Vcmpequw))",
            2,
        ),
        artifact_on(
            FindingKind::MetamorphicViolation,
            "PpuRecordCr6",
            "Ppu(Vx(Vcmpequw))",
            3,
        ),
        artifact_on(
            FindingKind::IllegalOutcome,
            "LegalOutcome",
            "Ppu(Ordinary(Stb))",
            4,
        ),
        artifact_on(
            FindingKind::IllegalOutcome,
            "LegalOutcome",
            "Ppu(Ordinary(Std))",
            5,
        ),
        artifact_on(
            FindingKind::IllegalOutcome,
            "LegalOutcome",
            "Ppu(Ordinary(Stfd))",
            6,
        ),
        artifact_on(
            FindingKind::IllegalOutcome,
            "LegalOutcome",
            "Ppu(Ordinary(Stmw))",
            7,
        ),
        artifact_on(
            FindingKind::IllegalOutcome,
            "LegalOutcome",
            "Ppu(Ordinary(Stw))",
            8,
        ),
    ];
    let text = render_campaign_summary(
        FuzzTarget::PpuInstruction,
        &summary,
        CampaignOutcome::Findings,
    );
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "{text}");
    // Six stores outrank two compares; five distinct store forms list
    // four in order and count the fifth; the replay is the earliest
    // artifact of the group.
    assert_eq!(
        lines[1],
        format!(
            "fuzz: findings=6 kind=IllegalOutcome check=LegalOutcome divergence=Outcome instructions=Ppu(Ordinary(Stb)),Ppu(Ordinary(Std)),Ppu(Ordinary(Stfd)),Ppu(Ordinary(Stmw)),+1 more replay: cellgov dev fuzz replay --artifact {}",
            PathBuf::from("out").join("1.json").display()
        )
    );
    assert_eq!(
        lines[2],
        format!(
            "fuzz: findings=2 kind=MetamorphicViolation check=PpuRecordCr6 divergence=Outcome instructions=Ppu(Vx(Vcmpequw)) replay: cellgov dev fuzz replay --artifact {}",
            PathBuf::from("out").join("2.json").display()
        )
    );
    assert_eq!(
        lines[3],
        format!(
            "fuzz: artifacts=8 stored=8 not_stored=0 dir={}",
            PathBuf::from("out").display()
        )
    );
}

#[test]
fn replay_semantic_and_raw_lines_render_from_their_records() {
    let replay = ReplayOutcome {
        case_index: 12,
        reduced: true,
        finding_kind: "IllegalOutcome".into(),
        fingerprint: fingerprint(),
        words: vec![0x3860_0006],
    };
    assert_eq!(
        render_replay_outcome(&replay),
        "fuzz replay: reproduced case=12 reduced=true kind=IllegalOutcome check=LegalOutcome divergence=Outcome words=[945815558]\n"
    );
    assert_eq!(replay.exit_code(), CommandExitCode::new(exit_codes::FAILED));
    assert_eq!(
        render_semantic_summary(&SemanticSummary {
            interpreter: "ppu",
            kinds: 10,
            witnesses: 9,
            findings: 0,
            refusals: 1,
        }),
        "fuzz semantic ppu: kinds=10 witnesses=9 findings=0 refusals=1\n"
    );
    assert_eq!(render_raw_progress(3, 10), "fuzz raw: 3 of 10 words");
    let raw = RawSummary {
        decoder: RawDecoder::Spu,
        status: RawDecodeStatus::Cancelled,
        processed: 3,
        domain: 10,
        accepted: 3,
        refused: 0,
        panics: 0,
        output: Some(PathBuf::from("raw.json")),
    };
    assert_eq!(
        render_raw_summary(&raw),
        "fuzz raw: Cancelled 3 of 10 words; accepted=3 refused=0 panics=0 -> raw.json\n"
    );
    assert_eq!(raw.exit_code(), CommandExitCode::new(EXIT_CANCELLED));
    let mut panicked = raw.clone();
    panicked.panics = 1;
    panicked.output = None;
    assert_eq!(
        render_raw_summary(&panicked),
        "fuzz raw: Cancelled 3 of 10 words; accepted=3 refused=0 panics=1\n"
    );
    assert_eq!(
        panicked.exit_code(),
        CommandExitCode::new(exit_codes::FAILED)
    );
    let mut complete = raw;
    complete.status = RawDecodeStatus::Complete;
    assert_eq!(complete.exit_code(), CommandExitCode::SUCCESS);
}
