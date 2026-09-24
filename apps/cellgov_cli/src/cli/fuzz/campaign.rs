//! Generated interpreter campaigns: the host a `cellgov_fuzz` campaign
//! runs under -- worker threads, the batch deadline, the progress bar and
//! the failure lines -- and the summary it prints.

use std::time::{Duration, Instant};

use cellgov_fuzz::artifact::ArtifactReference;
use cellgov_fuzz::runner::{
    run_campaign as run_plan, CampaignFailure, CampaignHost, CampaignPlan, CampaignRequest,
    WorkerFailure,
};
use cellgov_fuzz::{FuzzConfig, FuzzRun, FuzzTarget};
use cellgov_terminal::caps::{RenderFlags, RenderMode};
use cellgov_terminal::progress::{ProgressBar, ProgressSink};

use super::artifact::{read_reference, store_refusal};
use super::entry::{requested_workers, write_stdout};
use super::error::FuzzCliError;
use super::outcome::{render_campaign_summary, CampaignExit, CampaignOutcome};
use crate::cli::exit::CommandExitCode;
use crate::cli::parse::{FuzzCampaignArgs, FuzzCheck, FuzzReduction};
use crate::progress::FUZZ_CAMPAIGN_TASK;

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

/// A planned campaign and the host settings the library does not hold.
pub(super) struct PlannedCampaign {
    plan: CampaignPlan,
    timeout: Option<Duration>,
}

#[cfg(test)]
impl PlannedCampaign {
    pub(super) const fn limit(&self) -> u64 {
        self.plan.limit()
    }

    pub(super) const fn plan(&self) -> &CampaignPlan {
        &self.plan
    }
}

/// Refuses or accepts `args` for `target` before the run schedules a case; see
/// [`CampaignRequest::plan`].
pub(super) fn plan_campaign(
    args: &FuzzCampaignArgs,
    target: FuzzTarget,
) -> Result<PlannedCampaign, FuzzCliError> {
    if !matches!(args.check, FuzzCheck::All) {
        return Err(FuzzCliError::CheckUnavailable);
    }
    // The plan refuses the worker count and the deadline in their place
    // among its own checks, so an argv with several bad flags names the
    // same one it always did.
    let plan = CampaignRequest {
        target,
        campaign_version: args.campaign_version,
        seed: args.seed,
        strategy: args.strategy.into(),
        first: args.first,
        count: args.count,
        replay_case: args.replay_case,
        shard: args.shard,
        shards: args.shards,
        workers: requested_workers(args.workers),
        cancel_after: args.cancel_after,
        finding_limit: args.finding_limit,
        sequence_words: args.sequence_words,
        reduction: match args.reduction {
            FuzzReduction::None => None,
            FuzzReduction::OnFinding => Some(cellgov_fuzz::reduce::ReductionRequest {
                policy: args.reduction_policy.into(),
                budget: args.reduction_budget,
            }),
        },
        artifacts_dir: args.artifacts_dir.clone(),
        reference: ArtifactReference::Local,
        deadline_ms: args.deadline_ms,
        progress: args.progress,
    }
    .plan()?;
    let plan = match &args.reference {
        Some(path) => plan.with_reference(read_reference(path, target)?),
        None => plan,
    };
    Ok(PlannedCampaign {
        plan,
        timeout: args.deadline_ms.map(Duration::from_millis),
    })
}

/// The host a campaign runs under: worker threads, the deadline, the
/// progress bar, and the failure lines.
pub(super) struct CliHost<'a> {
    progress: &'a dyn ProgressSink,
    start: Instant,
    timeout: Option<Duration>,
    /// True while an in-place bar owns the terminal, so a stderr line
    /// waits in `diagnostics` until the bar is down. Otherwise the line
    /// prints at once: threshold lines interleave harmlessly, and an
    /// interrupt drops a held line.
    hold_diagnostics: bool,
    /// Failure lines held for stderr.
    diagnostics: Vec<String>,
    /// The first artifact failure, which becomes the command's result.
    artifact_failure: Option<FuzzCliError>,
}

impl<'a> CliHost<'a> {
    pub(super) fn new(
        progress: &'a dyn ProgressSink,
        timeout: Option<Duration>,
        hold: bool,
    ) -> Self {
        Self {
            progress,
            start: Instant::now(),
            timeout,
            hold_diagnostics: hold,
            diagnostics: Vec::new(),
            artifact_failure: None,
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

    /// Reports an artifact failure and keeps the first as the result.
    fn artifact_failed(&mut self, error: FuzzCliError) {
        self.report(format!("fuzz: {error}"));
        if self.artifact_failure.is_none() {
            self.artifact_failure = Some(error);
        }
    }
}

#[cfg(test)]
impl CliHost<'_> {
    pub(super) fn held(&self) -> &[String] {
        &self.diagnostics
    }

    pub(super) fn report_line(&mut self, line: &str) {
        self.report(line.to_owned());
    }
}

impl CampaignHost for CliHost<'_> {
    fn expired(&self) -> bool {
        self.timeout
            .is_some_and(|bound| self.start.elapsed() >= bound)
    }

    fn run_batch(
        &mut self,
        target: FuzzTarget,
        configs: Vec<FuzzConfig>,
    ) -> Result<Vec<FuzzRun>, WorkerFailure> {
        run_workers(
            configs
                .into_iter()
                .map(|config| move || target.run(config))
                .collect(),
        )
    }

    fn planned(&mut self, cases: u64) {
        self.progress.totals(0, cases);
    }

    fn batch_started(&mut self, first: u64, count: u64, findings: u64) {
        self.progress
            .item_started(&batch_label(first, count, findings));
    }

    fn batch_finished(&mut self, count: u64) {
        self.progress.advanced(count);
    }

    fn failed(&mut self, failure: CampaignFailure) {
        match failure {
            // The artifact records the refusal; the terminal names it too.
            CampaignFailure::Reduction { case_index, error } => self.report(format!(
                "fuzz: reduction of case {case_index} failed: {error}; original case kept"
            )),
            CampaignFailure::ArtifactBuild(error) => {
                self.artifact_failed(FuzzCliError::Artifact(error));
            }
            CampaignFailure::ArtifactStore {
                path,
                error,
                artifact,
            } => self.artifact_failed(store_refusal(&path, error, artifact)),
        }
    }
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
    target: FuzzTarget,
    render: RenderFlags,
) -> Result<CommandExitCode, FuzzCliError> {
    let planned = plan_campaign(args, target)?;
    // `--progress` enables the bar; the globals then decide how it
    // renders.
    let caps = RenderFlags {
        no_progress: render.no_progress || !args.progress,
        ..render
    }
    .caps();
    let bar = ProgressBar::start(caps, &FUZZ_CAMPAIGN_TASK, campaign_label(target));
    let sink = bar.sink();
    let mut host = CliHost::new(&*sink, planned.timeout, caps.mode == RenderMode::Ansi);
    let driven = run_plan(&planned.plan, &mut host);
    // Bar down first: the render thread owns stderr while it runs, and
    // its next frame moves the cursor up over the lines below. A
    // deadline or a failed batch leaves the bar short of its denominator.
    if driven
        .as_ref()
        .is_ok_and(|run| run.offset >= planned.plan.limit())
    {
        bar.finish();
    } else {
        bar.abort();
    }
    for line in &host.diagnostics {
        eprintln!("{line}");
    }
    let run = driven?;
    let outcome = run.outcome();
    write_stdout(&render_campaign_summary(target, &run.summary, outcome))?;
    // The error-backed outcomes carry their diagnostic through the typed
    // error, whose exit code is the outcome's.
    if let Some(error) = host.artifact_failure {
        return Err(error);
    }
    if let Some(source) = run.harness_failure {
        return Err(FuzzCliError::Harness(source));
    }
    if outcome == CampaignOutcome::NoEligibleCases {
        return Err(FuzzCliError::NoEligibleCases {
            cases: run.summary.cases,
            decoded: run.summary.decoded,
            unsupported: run.summary.unsupported,
            undefined: run.summary.undefined,
        });
    }
    Ok(outcome.exit_code())
}

/// Runs `work` on one scoped thread per job and returns the results in
/// order.
///
/// # Errors
///
/// [`WorkerFailure`] when a thread could not start or a job panicked.
pub(super) fn run_workers<T: Send, F: FnOnce() -> T + Send>(
    work: Vec<F>,
) -> Result<Vec<T>, WorkerFailure> {
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
            Err(WorkerFailure::Panicked)
        } else if let Some(source) = spawn_error {
            Err(WorkerFailure::Spawn(source))
        } else {
            Ok(runs)
        }
    })
}
