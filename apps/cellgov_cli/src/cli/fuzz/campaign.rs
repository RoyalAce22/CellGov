//! Generated interpreter campaigns: worker scheduling, batch deadlines,
//! finding reduction, and artifact storage.

use std::time::{Duration, Instant};

use cellgov_fuzz::artifact::{
    artifact_path, ArtifactCheckSelection, ArtifactExecutionPolicy, ArtifactFingerprint,
    ArtifactReduction, ArtifactReductionRequest, ArtifactReference, FuzzFindingArtifact,
};
use cellgov_fuzz::reduce::{reduce_finding, ReductionReport, ReductionRequest};
use cellgov_fuzz::report::Finding;
use cellgov_fuzz::{
    CampaignSchedule, CampaignShard, CampaignVersion, CaseRange, FuzzConfig, FuzzReport,
    FuzzTarget, ReductionOutcome, RunOutcome,
};
use cellgov_terminal::caps::{RenderFlags, RenderMode};
use cellgov_terminal::progress::{ProgressBar, ProgressSink};

use super::artifact::{persist_finding, read_reference};
use super::entry::{deadline, worker_count, write_stdout};
use super::error::FuzzCliError;
use super::outcome::{render_campaign_summary, ArtifactRecord, CampaignOutcome, CampaignSummary};
use crate::cli::exit::CommandExitCode;
use crate::cli::parse::{FuzzCampaignArgs, FuzzCheck, FuzzReduction};
use crate::progress::FUZZ_CAMPAIGN_TASK;

const CAMPAIGN_BATCH_CASES: u64 = 64;

/// The subcommand's name, as the progress bar labels the campaign.
pub(super) const fn campaign_label(target: FuzzTarget) -> &'static str {
    match target {
        FuzzTarget::PpuInstruction => "ppu-instruction",
        FuzzTarget::PpuSequence => "ppu-sequence",
        FuzzTarget::SpuInstruction => "spu-instruction",
        FuzzTarget::SpuSequence => "spu-sequence",
    }
}

/// Refuses a `--finding-limit` outside what an engine retains.
pub(super) fn check_finding_limit(limit: u32) -> Result<(), FuzzCliError> {
    if limit == 0 || limit > cellgov_fuzz::MAX_RETAINED_FINDINGS {
        return Err(FuzzCliError::FindingLimit);
    }
    Ok(())
}

/// A campaign's host settings, after every refusal check passes.
pub(super) struct CampaignPlan<'a> {
    args: &'a FuzzCampaignArgs,
    engine: FuzzTarget,
    /// First case index the schedule considers.
    first: u64,
    /// Case indices the schedule declares.
    count: u64,
    /// Case indices the loop considers: `count`, or `--cancel-after`.
    limit: u64,
    workers: usize,
    workers_u32: u32,
    total_shards: u32,
    timeout: Option<Duration>,
    campaign_config: FuzzConfig,
    reduction: Option<ReductionRequest>,
    artifacts_dir: &'a str,
    reference: ArtifactReference,
}

#[cfg(test)]
impl CampaignPlan<'_> {
    pub(super) const fn limit(&self) -> u64 {
        self.limit
    }
}

/// What the batch loop accumulates.
#[derive(Default)]
pub(super) struct CampaignRun {
    summary: CampaignSummary,
    /// Case indices considered so far.
    offset: u64,
    harness_failure: Option<cellgov_fuzz::FuzzError>,
    artifact_failure: Option<FuzzCliError>,
    artifact_index: u64,
    /// True while an in-place bar owns the terminal, so a stderr line
    /// waits in `diagnostics` until the bar is down. Otherwise the line
    /// prints at once: threshold lines interleave harmlessly, and an
    /// interrupt drops a held line.
    hold_diagnostics: bool,
    /// Failure lines held for stderr.
    diagnostics: Vec<String>,
}

impl CampaignRun {
    fn holding(hold: bool) -> Self {
        Self {
            hold_diagnostics: hold,
            ..Self::default()
        }
    }

    /// One failure line for stderr.
    fn report(&mut self, line: String) {
        if self.hold_diagnostics {
            self.diagnostics.push(line);
        } else {
            eprintln!("{line}");
        }
    }
}

#[cfg(test)]
impl CampaignRun {
    pub(super) const fn offset(&self) -> u64 {
        self.offset
    }

    pub(super) fn held(&self) -> &[String] {
        &self.diagnostics
    }

    pub(super) fn report_under(hold: bool, line: &str) -> Self {
        let mut run = Self::holding(hold);
        run.report(line.to_owned());
        run
    }
}

/// Refuse or accept `args` for `engine` before the loop schedules a case.
pub(super) fn plan_campaign<'a>(
    args: &'a FuzzCampaignArgs,
    engine: FuzzTarget,
) -> Result<CampaignPlan<'a>, FuzzCliError> {
    // [Manes2021 p:3 s:2.3 Fuzz Testing Algorithm] The model fuzzer runs one preprocessing step before its iteration loop. Every refusal below lands in that step, before the loop schedules the first case.
    if !matches!(args.check, FuzzCheck::All) {
        return Err(FuzzCliError::CheckUnavailable);
    }
    let reduction = match args.reduction {
        FuzzReduction::None => None,
        FuzzReduction::OnFinding => Some(ReductionRequest {
            policy: args.reduction_policy.into(),
            budget: args.reduction_budget,
        }),
    };
    if reduction.is_some_and(|request| request.budget == 0) {
        return Err(FuzzCliError::Invalid("reduction-budget must be positive"));
    }
    check_finding_limit(args.finding_limit)?;
    if !engine.generates_sequences() && args.sequence_words.is_some() {
        return Err(FuzzCliError::Invalid(
            "sequence-words applies only to sequence campaigns",
        ));
    }
    if args.shards == 0 || args.shard >= args.shards {
        return Err(FuzzCliError::Invalid(
            "shard must be below a positive shards count",
        ));
    }
    // `FuzzFindingArtifact::from_finding` refuses a non-UTF-8 replay path
    // only after the campaign ran, and its error carries no finding. This
    // check refuses the directory before the campaign schedules a case.
    let artifacts_dir = args
        .artifacts_dir
        .to_str()
        .ok_or(FuzzCliError::Invalid("artifacts-dir must be valid UTF-8"))?
        .trim_end_matches(['/', '\\']);
    if artifacts_dir.is_empty() {
        return Err(FuzzCliError::Invalid("artifacts-dir must not be empty"));
    }
    let (first, count) = args
        .replay_case
        .map_or((args.first, args.count), |index| (index, 1));
    if count == 0 || first.checked_add(count - 1).is_none() {
        return Err(FuzzCliError::Range { first, count });
    }
    let limit = args.cancel_after.unwrap_or(count);
    if limit > count {
        return Err(FuzzCliError::Invalid("cancel-after exceeds the case count"));
    }
    let workers = worker_count(args.workers)?;
    let workers_u32 =
        u32::try_from(workers).map_err(|_| FuzzCliError::Invalid("workers exceed u32"))?;
    let total_shards = args
        .shards
        .checked_mul(workers_u32)
        .ok_or(FuzzCliError::Invalid("shards times workers overflows"))?;
    let timeout = deadline(args.deadline_ms)?;
    let campaign_config = FuzzConfig {
        campaign_version: CampaignVersion(args.campaign_version),
        seed: args.seed,
        strategy: args.strategy.into(),
        schedule: CampaignSchedule {
            cases: CaseRange { first, count },
            shard: CampaignShard {
                index: args.shard,
                count: args.shards,
            },
            cancellation: args.cancel_after.map(cellgov_fuzz::CancellationBoundary),
        },
        retention: Default::default(),
        max_findings: args.finding_limit,
        sequence_words: args
            .sequence_words
            .unwrap_or(cellgov_fuzz::DEFAULT_SEQUENCE_WORDS),
    };
    campaign_config.validate_for_target(engine)?;
    let reference = if let Some(path) = &args.reference {
        read_reference(path, engine)?
    } else {
        ArtifactReference::Local
    };
    Ok(CampaignPlan {
        args,
        engine,
        first,
        count,
        limit,
        workers,
        workers_u32,
        total_shards,
        timeout,
        campaign_config,
        reduction,
        artifacts_dir,
        reference,
    })
}

/// Run the batches of `plan` and report each one to `progress`.
///
/// A failure line goes through [`CampaignRun::report`]. After the bar
/// is down, the caller prints the lines `state` held.
pub(super) fn drive(
    plan: &CampaignPlan<'_>,
    state: &mut CampaignRun,
    progress: &dyn ProgressSink,
) -> Result<(), FuzzCliError> {
    let args = plan.args;
    let engine = plan.engine;
    let start = Instant::now();
    progress.totals(0, plan.limit);
    // [Manes2021 p:3 s:2.3 Fuzz Testing Algorithm] The model loop iterates until its time limit passes or its continue check says stop; this loop keeps both exits.
    while state.offset < plan.limit {
        if plan.timeout.is_some_and(|bound| start.elapsed() >= bound) {
            break;
        }
        let batch = (plan.limit - state.offset).min(CAMPAIGN_BATCH_CASES);
        let batch_first = plan
            .first
            .checked_add(state.offset)
            .ok_or(FuzzCliError::Range {
                first: plan.first,
                count: plan.count,
            })?;
        progress.item_started(&batch_label(batch_first, batch, state.summary.findings()));
        let shift = (state.offset % u64::from(plan.total_shards)) as u32;
        let mut work = Vec::with_capacity(plan.workers);
        for worker in 0..plan.workers_u32 {
            let local_shard =
                worker_shard_index(args.shard, args.shards, worker, plan.total_shards, shift)?;
            let config = FuzzConfig {
                campaign_version: CampaignVersion(args.campaign_version),
                seed: args.seed,
                strategy: args.strategy.into(),
                schedule: CampaignSchedule {
                    cases: CaseRange {
                        first: batch_first,
                        count: batch,
                    },
                    shard: CampaignShard {
                        index: local_shard,
                        count: plan.total_shards,
                    },
                    cancellation: None,
                },
                retention: Default::default(),
                max_findings: args.finding_limit,
                sequence_words: args
                    .sequence_words
                    .unwrap_or(cellgov_fuzz::DEFAULT_SEQUENCE_WORDS),
            };
            work.push(move || engine.run(config));
        }
        let runs = run_workers(work)?;
        for run in runs {
            for finding in &run.report.findings {
                let path = artifact_path(
                    plan.artifacts_dir,
                    &format!(
                        "{engine:?}-{:?}-{}",
                        plan.campaign_config.strategy, args.seed
                    ),
                    finding.replay.case_index,
                    state.artifact_index,
                );
                let mut finding = finding.clone();
                if let Some(request) = plan.reduction {
                    finding.reduction =
                        reduce_retained_finding(plan.campaign_config, &finding, request);
                    // The artifact records the refusal; the terminal names it too,
                    // like the artifact write failures below.
                    if let ReductionOutcome::Failed(error) = &finding.reduction {
                        state.summary.reductions_failed = state
                            .summary
                            .reductions_failed
                            .checked_add(1)
                            .ok_or(FuzzCliError::CounterOverflow)?;
                        state.report(format!(
                            "fuzz: reduction of case {} failed: {error}; original case kept",
                            finding.replay.case_index
                        ));
                    }
                }
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
                    plan.campaign_config,
                    ArtifactExecutionPolicy {
                        workers: plan.workers_u32,
                        deadline_ms: args.deadline_ms,
                        progress: args.progress,
                        check: ArtifactCheckSelection::All,
                        reduction: plan.reduction.map_or(
                            ArtifactReductionRequest::None,
                            |request| ArtifactReductionRequest::OnFinding {
                                policy: request.policy,
                                budget: request.budget,
                            },
                        ),
                    },
                    &run.report,
                    &finding,
                    plan.reference.clone(),
                    &path,
                )
                .map_err(FuzzCliError::from)
                .and_then(|artifact| persist_finding(&path, artifact));
                // A failed write stops neither the findings after it nor the
                // summary line. The loop keeps every failure with its retained
                // evidence, and the first failure becomes the command's result.
                match stored {
                    Ok(()) => record.stored = true,
                    Err(error) => {
                        state.report(format!("fuzz: {error}"));
                        if state.artifact_failure.is_none() {
                            state.artifact_failure = Some(error);
                        }
                    }
                }
                state.summary.artifacts.push(record);
                state.artifact_index = state
                    .artifact_index
                    .checked_add(1)
                    .ok_or(FuzzCliError::CounterOverflow)?;
            }
            accumulate(&mut state.summary, &run.report)?;
            if let RunOutcome::HarnessFailure(source) = &run.outcome {
                if state.harness_failure.is_none() {
                    state.harness_failure = Some(source.clone());
                }
            }
        }
        state.offset += batch;
        progress.advanced(batch);
    }
    Ok(())
}

/// The bar's current-item text for one batch.
fn batch_label(first: u64, batch: u64, findings: u64) -> String {
    let last = first + (batch - 1);
    if findings == 0 {
        format!("cases {first}..={last}")
    } else {
        format!("cases {first}..={last}, {findings} findings")
    }
}

pub(super) fn run_campaign(
    args: &FuzzCampaignArgs,
    engine: FuzzTarget,
    render: RenderFlags,
) -> Result<CommandExitCode, FuzzCliError> {
    let plan = plan_campaign(args, engine)?;
    // `--progress` enables the bar; the globals then decide how it
    // renders.
    let caps = RenderFlags {
        no_progress: render.no_progress || !args.progress,
        ..render
    }
    .caps();
    let mut state = CampaignRun::holding(caps.mode == RenderMode::Ansi);
    let bar = ProgressBar::start(caps, &FUZZ_CAMPAIGN_TASK, campaign_label(engine));
    let sink = bar.sink();
    let driven = drive(&plan, &mut state, &*sink);
    // Bar down first: the render thread owns stderr while it runs, and
    // its next frame moves the cursor up over the lines below. A
    // deadline or a failed batch leaves the bar short of its denominator.
    if driven.is_ok() && state.offset >= plan.limit {
        bar.finish();
    } else {
        bar.abort();
    }
    for line in &state.diagnostics {
        eprintln!("{line}");
    }
    driven?;
    let CampaignRun {
        mut summary,
        offset,
        harness_failure,
        artifact_failure,
        ..
    } = state;
    summary.cancelled = offset < plan.count;
    let outcome = CampaignOutcome::classify(
        &summary,
        artifact_failure.is_some(),
        harness_failure.is_some(),
    );
    write_stdout(&render_campaign_summary(engine, &summary, outcome))?;
    // The error-backed outcomes carry their diagnostic through the typed
    // error, whose exit code is the outcome's.
    if let Some(error) = artifact_failure {
        return Err(error);
    }
    if let Some(source) = harness_failure {
        return Err(FuzzCliError::Harness(source));
    }
    if outcome == CampaignOutcome::NoEligibleCases {
        return Err(FuzzCliError::NoEligibleCases {
            cases: summary.cases,
            decoded: summary.decoded,
            unsupported: summary.unsupported,
            undefined: summary.undefined,
        });
    }
    Ok(outcome.exit_code())
}

fn accumulate(summary: &mut CampaignSummary, report: &FuzzReport) -> Result<(), FuzzCliError> {
    let add = |total: &mut u64, count: u64| {
        *total = total
            .checked_add(count)
            .ok_or(FuzzCliError::CounterOverflow)?;
        Ok::<(), FuzzCliError>(())
    };
    add(&mut summary.cases, report.cases)?;
    add(&mut summary.decoded, report.decoded)?;
    add(&mut summary.eligible, report.eligible_cases)?;
    add(&mut summary.unsupported, report.unsupported_cases)?;
    add(&mut summary.undefined, report.undefined_cases)?;
    for (kind, count) in &report.finding_counts {
        add(summary.finding_counts.entry(*kind).or_insert(0), *count)?;
    }
    Ok(())
}

pub(super) fn run_workers<T: Send, F: FnOnce() -> T + Send>(
    work: Vec<F>,
) -> Result<Vec<T>, FuzzCliError> {
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(work.len());
        let mut spawn_error = None;
        for job in work {
            match std::thread::Builder::new().spawn_scoped(scope, job) {
                Ok(handle) => handles.push(handle),
                Err(source) => {
                    spawn_error = Some(source);
                    break;
                }
            }
        }
        let mut runs = Vec::with_capacity(handles.len());
        let mut worker_panicked = false;
        for handle in handles {
            match handle.join() {
                Ok(run) => runs.push(run),
                Err(_) => worker_panicked = true,
            }
        }
        if worker_panicked {
            Err(FuzzCliError::WorkerPanic)
        } else if let Some(source) = spawn_error {
            Err(FuzzCliError::WorkerSpawn(source))
        } else {
            Ok(runs)
        }
    })
}

pub(super) fn worker_shard_index(
    shard: u32,
    shards: u32,
    worker: u32,
    total_shards: u32,
    batch_shift: u32,
) -> Result<u32, FuzzCliError> {
    let absolute = worker
        .checked_mul(shards)
        .and_then(|value| value.checked_add(shard))
        .ok_or(FuzzCliError::Invalid("worker shard index overflows"))?;
    if absolute >= total_shards || batch_shift >= total_shards {
        return Err(FuzzCliError::Invalid(
            "worker shard index is outside the partition",
        ));
    }
    Ok(if absolute >= batch_shift {
        absolute - batch_shift
    } else {
        total_shards - (batch_shift - absolute)
    })
}

pub(super) fn reduce_retained_finding(
    config: FuzzConfig,
    finding: &Finding,
    request: ReductionRequest,
) -> ReductionOutcome {
    reduce_finding(config, finding, request)
        .map_or_else(ReductionOutcome::Failed, ReductionReport::into_outcome)
}
