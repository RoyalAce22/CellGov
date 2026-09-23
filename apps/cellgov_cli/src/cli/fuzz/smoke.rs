//! The bounded smoke set: every campaign the library declares, run once,
//! every finding minimized and stored, and each held to a promoted
//! regression.

use std::path::PathBuf;

use cellgov_fuzz::artifact::{
    ArtifactCheckSelection, ArtifactExecutionPolicy, ArtifactFingerprint, ArtifactReduction,
    ArtifactReductionRequest, ArtifactReference, FuzzFindingArtifact,
};
use cellgov_fuzz::reduce::{ReductionPolicy, ReductionRequest};
use cellgov_fuzz::regression::{self, Regression, RegressionProfile};
use cellgov_fuzz::smoke::SMOKE_CAMPAIGNS;
use cellgov_fuzz::{FindingKind, ReductionOutcome, RunOutcome};

use super::artifact::persist_finding;
use super::campaign::reduce_retained_finding;
use super::entry::{reports_progress, write_stdout};
use super::error::FuzzCliError;
use super::outcome::{
    render_promotion, render_smoke_campaign, render_smoke_finding, render_smoke_outcome,
    ArtifactRecord, SmokeCampaignSummary, SmokeOutcome,
};
use crate::cli::exit::CommandExitCode;
use crate::cli::parse::{FuzzPromoteArgs, FuzzRegressionProfile, FuzzSmokeArgs};

/// Promotes one stored artifact into a regression directory as open.
pub(super) fn run_promote(args: &FuzzPromoteArgs) -> Result<CommandExitCode, FuzzCliError> {
    let json =
        std::fs::read_to_string(&args.artifact).map_err(|source| FuzzCliError::ArtifactRead {
            path: args.artifact.clone(),
            source,
        })?;
    let artifact = FuzzFindingArtifact::parse_json(&json)?;
    let profile = match args.profile {
        FuzzRegressionProfile::Both => RegressionProfile::Both,
        FuzzRegressionProfile::Debug => RegressionProfile::Debug,
        FuzzRegressionProfile::Release => RegressionProfile::Release,
    };
    let promoted = regression::promote(
        &args.regressions,
        &args.name,
        profile,
        &args.summary,
        &artifact,
    )?;
    write_stdout(&render_promotion(&promoted))?;
    Ok(CommandExitCode::SUCCESS)
}

/// Runs the smoke set: one campaign after another, each finding reduced and
/// stored, each held to the regressions the caller named.
///
/// The regressions load before the first campaign runs, so a directory that
/// cannot stand as a witness costs no campaign. Every campaign runs whatever
/// the earlier ones found. The outcome ranks what the set as a whole found.
pub(super) fn run_smoke(
    args: &FuzzSmokeArgs,
    quiet: bool,
) -> Result<CommandExitCode, FuzzCliError> {
    let artifacts_dir = args
        .artifacts_dir
        .to_str()
        .ok_or(FuzzCliError::Invalid("artifacts-dir must be valid UTF-8"))?;
    if artifacts_dir.is_empty() {
        return Err(FuzzCliError::Invalid("artifacts-dir must not be empty"));
    }
    // An artifact path is portable text, spelled as `regression::promote`
    // spells a stored replay path: the directory as the caller gave it, one
    // forward slash, then the file.
    let artifacts_dir = artifacts_dir.trim_end_matches(['/', '\\']);
    if args.reduction_budget == 0 {
        return Err(FuzzCliError::Invalid("reduction-budget must be positive"));
    }
    let regressions: Vec<Regression> = match &args.regressions {
        Some(dir) => regression::load(dir)?,
        None => Vec::new(),
    };
    let request = ReductionRequest {
        policy: ReductionPolicy::Deterministic,
        budget: args.reduction_budget,
    };
    let mut evidence_not_stored = None;
    let mut harness_failure = None;
    let mut reductions_failed = 0u64;
    let mut unpromoted_total = 0u64;
    let mut vacuous = false;
    for campaign in &SMOKE_CAMPAIGNS {
        let config = campaign.config();
        let run = campaign.run();
        let mut promoted = 0u64;
        let mut unpromoted = 0u64;
        for (index, finding) in run.report.findings.iter().enumerate() {
            let mut finding = finding.clone();
            finding.reduction = reduce_retained_finding(config, &finding, request);
            if let ReductionOutcome::Failed(error) = &finding.reduction {
                reductions_failed = reductions_failed
                    .checked_add(1)
                    .ok_or(FuzzCliError::CounterOverflow)?;
                eprintln!(
                    "fuzz smoke: reduction of {} case {} failed: {error}; original case kept",
                    campaign.name, finding.replay.case_index
                );
            }
            let path = PathBuf::from(format!(
                "{artifacts_dir}/{}-{}-{index}.json",
                campaign.name, finding.replay.case_index
            ));
            let mut record = ArtifactRecord {
                path: path.clone(),
                campaign_version: finding.replay.campaign_version.0,
                seed: finding.replay.seed,
                case_index: finding.replay.case_index,
                finding_kind: finding.kind,
                fingerprint: ArtifactFingerprint::from(&finding.fingerprint),
                reduction: ArtifactReduction::from(&finding.reduction),
                stored: false,
            };
            let stored = FuzzFindingArtifact::from_finding(
                config,
                ArtifactExecutionPolicy {
                    workers: 1,
                    deadline_ms: None,
                    progress: args.progress,
                    check: ArtifactCheckSelection::All,
                    reduction: ArtifactReductionRequest::OnFinding {
                        policy: request.policy,
                        budget: request.budget,
                    },
                },
                &run.report,
                &finding,
                ArtifactReference::Local,
                &path,
            )
            .map_err(FuzzCliError::from)
            .and_then(|artifact| persist_finding(&path, artifact));
            match stored {
                Ok(()) => record.stored = true,
                Err(error) => {
                    eprintln!("fuzz smoke: {error}");
                    if evidence_not_stored.is_none() {
                        evidence_not_stored = Some(error);
                    }
                }
            }
            let covering = regression::promoted(&regressions, &finding);
            match covering {
                Some(_) => promoted += 1,
                None => unpromoted += 1,
            }
            write_stdout(&render_smoke_finding(
                campaign.name,
                &record,
                covering.map(|regression| regression.entry.name.as_str()),
            ))?;
        }
        let covered = campaign.check_coverage(&run.report);
        if let Err(error) = &covered {
            vacuous = true;
            eprintln!("fuzz smoke: {error}");
        }
        if let RunOutcome::HarnessFailure(source) = &run.outcome {
            eprintln!(
                "fuzz smoke: {} failed inside the harness: {source}",
                campaign.name
            );
            if harness_failure.is_none() {
                harness_failure = Some(source.clone());
            }
        }
        // The engine retains a bounded number of findings and counts the
        // rest. A counted finding the engine did not retain met no
        // regression, so it ranks as unpromoted.
        let retained = run.report.findings.len() as u64;
        let findings = run
            .report
            .finding_counts
            .iter()
            .filter(|(kind, _)| !matches!(kind, FindingKind::Unsupported | FindingKind::Undefined))
            .fold(0u64, |total, (_, count)| total.saturating_add(*count));
        unpromoted = unpromoted.saturating_add(findings.saturating_sub(retained));
        unpromoted_total = unpromoted_total
            .checked_add(unpromoted)
            .ok_or(FuzzCliError::CounterOverflow)?;
        let summary = SmokeCampaignSummary {
            name: campaign.name,
            campaign_version: config.campaign_version.0,
            seed: campaign.seed,
            cases: run.report.cases,
            eligible: run.report.eligible_cases,
            instruction_kinds: run.report.instruction_kinds.len() as u64,
            findings,
            retained,
            promoted,
            unpromoted,
            covered: covered.is_ok(),
        };
        let line = render_smoke_campaign(&summary);
        if reports_progress(args.progress, quiet) {
            eprint!("{line}");
        }
        write_stdout(&line)?;
    }
    let outcome = SmokeOutcome::classify(
        evidence_not_stored.is_some(),
        harness_failure.is_some(),
        reductions_failed,
        unpromoted_total,
        vacuous,
    );
    write_stdout(&render_smoke_outcome(SMOKE_CAMPAIGNS.len(), outcome))?;
    if let Some(error) = evidence_not_stored {
        return Err(error);
    }
    if let Some(source) = harness_failure {
        return Err(FuzzCliError::Harness(source));
    }
    Ok(outcome.exit_code())
}
