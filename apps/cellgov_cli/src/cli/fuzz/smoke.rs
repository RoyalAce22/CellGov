//! The bounded smoke set: every campaign the library declares, run once,
//! every finding minimized and stored, and each held to a promoted
//! regression. With a reference directory, the set also replays the SPU
//! reference tier.

use cellgov_fuzz::artifact::{
    artifact_path, ArtifactCheckSelection, ArtifactExecutionPolicy, ArtifactFingerprint,
    ArtifactReduction, ArtifactReductionRequest, ArtifactReference, FuzzFindingArtifact,
};
use cellgov_fuzz::reduce::{ReductionPolicy, ReductionRequest};
use cellgov_fuzz::regression::{self, Regression, RegressionProfile};
use cellgov_fuzz::report::{failing_findings, Finding, FuzzRun};
use cellgov_fuzz::runner::reduce_retained_finding;
use cellgov_fuzz::smoke::{SmokeCampaign, SmokeError, SMOKE_CAMPAIGNS, SMOKE_REFERENCE};
use cellgov_fuzz::spu_reference::SpuReferenceCampaign;
use cellgov_fuzz::{FuzzConfig, FuzzError, ReductionOutcome, RunOutcome};

use super::artifact::persist_finding;
use super::entry::{reports_progress, write_stdout};
use super::error::FuzzCliError;
use super::outcome::{
    render_promotion, render_smoke_campaign, render_smoke_finding, render_smoke_outcome,
    render_smoke_reference, ArtifactRecord, SmokeCampaignSummary, SmokeOutcome,
    SmokeReferenceSummary,
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
/// stored, each held to the regressions the caller named. With a reference
/// directory, the set also replays it as its reference tier.
///
/// The set loads the regressions and replays the reference directory before
/// the first campaign runs, so a directory that cannot stand as a witness
/// costs no campaign. Every campaign runs whatever the earlier ones found.
/// The outcome ranks what the set as a whole found.
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
    if args.reduction_budget == 0 {
        return Err(FuzzCliError::Invalid("reduction-budget must be positive"));
    }
    let regressions: Vec<Regression> = match &args.regressions {
        Some(dir) => regression::load(dir)?,
        None => Vec::new(),
    };
    let reference = args
        .spu_references
        .as_ref()
        .map(|dir| SMOKE_REFERENCE.run(dir))
        .transpose()?;
    let store = FindingStore {
        artifacts_dir,
        progress: args.progress,
        request: ReductionRequest {
            policy: ReductionPolicy::Deterministic,
            budget: args.reduction_budget,
        },
        regressions: &regressions,
    };
    let mut tally = SmokeTally::default();
    for campaign in &SMOKE_CAMPAIGNS {
        let config = campaign.config();
        let run = campaign.run();
        let mut promoted = 0u64;
        for (index, finding) in run.report.findings.iter().enumerate() {
            promoted += u64::from(store.store(campaign, config, &run, index, finding, &mut tally)?);
        }
        let summary = summarize_campaign(campaign, config, &run, promoted, &mut tally)?;
        let line = render_smoke_campaign(&summary);
        if reports_progress(args.progress, quiet) {
            eprint!("{line}");
        }
        write_stdout(&line)?;
    }
    let mut campaigns = SMOKE_CAMPAIGNS.len();
    if let Some(replay) = &reference {
        campaigns += 1;
        let line = render_smoke_reference(&replay_reference_tier(replay, &mut tally)?);
        if reports_progress(args.progress, quiet) {
            eprint!("{line}");
        }
        write_stdout(&line)?;
    }
    let outcome = SmokeOutcome::classify(
        tally.evidence_not_stored.is_some(),
        tally.harness_failure.is_some(),
        tally.reductions_failed,
        tally.unpromoted_total,
        tally.vacuous,
    );
    write_stdout(&render_smoke_outcome(campaigns, outcome))?;
    if let Some(error) = tally.evidence_not_stored {
        return Err(error);
    }
    if let Some(source) = tally.harness_failure {
        return Err(FuzzCliError::Harness(source));
    }
    Ok(outcome.exit_code())
}

/// What the smoke set counts across its campaigns and its reference tier.
#[derive(Default)]
struct SmokeTally {
    /// The first finding whose evidence the set could not store.
    evidence_not_stored: Option<FuzzCliError>,
    /// The first campaign that failed inside the harness.
    harness_failure: Option<FuzzError>,
    reductions_failed: u64,
    unpromoted_total: u64,
    vacuous: bool,
}

impl SmokeTally {
    fn add_unpromoted(&mut self, count: u64) -> Result<(), FuzzCliError> {
        self.unpromoted_total = self
            .unpromoted_total
            .checked_add(count)
            .ok_or(FuzzCliError::CounterOverflow)?;
        Ok(())
    }
}

/// Where and how the smoke set reduces, stores and ranks each finding.
struct FindingStore<'a> {
    artifacts_dir: &'a str,
    progress: bool,
    request: ReductionRequest,
    regressions: &'a [Regression],
}

impl FindingStore<'_> {
    /// Reduce the `index`th finding of `run`, store it, and print its
    /// line; returns whether a regression covers it.
    fn store(
        &self,
        campaign: &SmokeCampaign,
        config: FuzzConfig,
        run: &FuzzRun,
        index: usize,
        finding: &Finding,
        tally: &mut SmokeTally,
    ) -> Result<bool, FuzzCliError> {
        let mut finding = finding.clone();
        finding.reduction = reduce_retained_finding(config, &finding, self.request);
        if let ReductionOutcome::Failed(error) = &finding.reduction {
            tally.reductions_failed = tally
                .reductions_failed
                .checked_add(1)
                .ok_or(FuzzCliError::CounterOverflow)?;
            eprintln!(
                "fuzz smoke: reduction of {} case {} failed: {error}; original case kept",
                campaign.name, finding.replay.case_index
            );
        }
        let path = artifact_path(
            self.artifacts_dir,
            campaign.name,
            finding.replay.case_index,
            index as u64,
        );
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
                progress: self.progress,
                check: ArtifactCheckSelection::All,
                reduction: ArtifactReductionRequest::OnFinding {
                    policy: self.request.policy,
                    budget: self.request.budget,
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
                tally.evidence_not_stored.get_or_insert(error);
            }
        }
        let covering = regression::promoted(self.regressions, &finding);
        write_stdout(&render_smoke_finding(
            campaign.name,
            &record,
            covering.map(|regression| regression.entry.name.as_str()),
        ))?;
        Ok(covering.is_some())
    }
}

/// The summary line of one campaign whose retained findings `promoted`
/// regressions cover, after its coverage and harness checks.
fn summarize_campaign(
    campaign: &SmokeCampaign,
    config: FuzzConfig,
    run: &FuzzRun,
    promoted: u64,
    tally: &mut SmokeTally,
) -> Result<SmokeCampaignSummary, FuzzCliError> {
    let covered = campaign.check_coverage(&run.report);
    if let Err(error) = &covered {
        tally.vacuous = true;
        eprintln!("fuzz smoke: {error}");
    }
    if let RunOutcome::HarnessFailure(source) = &run.outcome {
        eprintln!(
            "fuzz smoke: {} failed inside the harness: {source}",
            campaign.name
        );
        tally.harness_failure.get_or_insert_with(|| source.clone());
    }
    // The engine retains a bounded number of findings and counts the
    // rest. A counted finding the engine did not retain met no
    // regression, so it ranks as unpromoted.
    let retained = run.report.findings.len() as u64;
    let findings = failing_findings(&run.report.finding_counts);
    let unpromoted = (retained - promoted).saturating_add(findings.saturating_sub(retained));
    tally.add_unpromoted(unpromoted)?;
    Ok(SmokeCampaignSummary {
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
    })
}

/// Check the reference tier's replay and tally what it found.
fn replay_reference_tier(
    replay: &SpuReferenceCampaign,
    tally: &mut SmokeTally,
) -> Result<SmokeReferenceSummary, FuzzCliError> {
    let checked = SMOKE_REFERENCE.check(replay);
    if let Err(error) = &checked {
        eprintln!("fuzz smoke: {error}");
    }
    // A replay that is not clean is a finding no regression covers,
    // whatever error the check names first.
    if !replay.is_clean() {
        tally.add_unpromoted(1)?;
    }
    let covered = SMOKE_REFERENCE.reaches_floor(replay)
        && !matches!(checked, Err(SmokeError::Unpinned { .. }));
    tally.vacuous |= !covered;
    Ok(SmokeReferenceSummary {
        name: SMOKE_REFERENCE.name,
        files: replay.files.len() as u64,
        vectors: replay.vectors() as u64,
        failing: replay
            .files
            .iter()
            .filter(|run| !run.outcome.is_match())
            .count() as u64,
        gaps: replay.completeness.gaps() as u64,
        covered,
    })
}
