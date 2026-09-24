//! The multi-batch campaign runner. It plans one generated campaign,
//! schedules it over bounded batches of worker shards, reduces and
//! stores each retained finding, and classifies the whole run.
//!
//! The caller owns the host: it runs each batch's worker configurations
//! (in parallel or not), keeps the clock that ends a run early, shows
//! progress, and decides when a failure line prints. It sees all of it
//! through [`CampaignHost`].

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::artifact::{
    artifact_path, ArtifactCheckSelection, ArtifactError, ArtifactExecutionPolicy,
    ArtifactFingerprint, ArtifactReduction, ArtifactReductionRequest, ArtifactReference,
    ArtifactStoreError, FuzzFindingArtifact,
};
use crate::reduce::{reduce_finding, ReductionReport, ReductionRequest};
use crate::report::{failing_findings, Finding, FindingKind, FuzzReport, FuzzRun};
use crate::{
    CampaignSchedule, CampaignShard, CampaignVersion, CancellationBoundary, CaseRange,
    ConfigurationError, FuzzConfig, FuzzError, FuzzTarget, GenerationStrategy, ReductionError,
    ReductionOutcome, RunOutcome, DEFAULT_SEQUENCE_WORDS, MAX_RETAINED_FINDINGS,
};

/// Case indices one batch considers at most.
pub const CAMPAIGN_BATCH_CASES: u64 = 64;

/// Workers a campaign may split each batch across.
pub const MAX_CAMPAIGN_WORKERS: usize = 64;

/// One campaign as the caller asks for it, before any refusal check.
#[derive(Debug, Clone)]
pub struct CampaignRequest {
    /// Engine the campaign runs.
    pub target: FuzzTarget,
    /// Generator version the campaign replays under.
    pub campaign_version: u32,
    /// Master seed.
    pub seed: u64,
    /// How the generator builds each case.
    pub strategy: GenerationStrategy,
    /// First case index of the range.
    pub first: u64,
    /// Case indices the range declares.
    pub count: u64,
    /// A single case index to run in place of the range.
    pub replay_case: Option<u64>,
    /// Zero-based shard of the range this invocation owns.
    pub shard: u32,
    /// Shards the range is split into.
    pub shards: u32,
    /// Workers each batch is split across.
    pub workers: usize,
    /// Offset in the range at which the run stops early.
    pub cancel_after: Option<u64>,
    /// Detailed findings each worker run retains.
    pub finding_limit: u32,
    /// Instruction words per sequence case; `None` takes the default.
    pub sequence_words: Option<u32>,
    /// How to reduce each retained finding, if at all.
    pub reduction: Option<ReductionRequest>,
    /// Directory the run writes each finding's artifact under. It must be
    /// UTF-8, since every artifact records its own replay path.
    pub artifacts_dir: PathBuf,
    /// Independent reference every artifact carries.
    pub reference: ArtifactReference,
    /// The caller's deadline, as the artifact records it.
    pub deadline_ms: Option<u64>,
    /// Whether the caller shows progress, as the artifact records it.
    pub progress: bool,
}

/// Why the plan refused a campaign, or why its run stopped.
#[derive(Debug, thiserror::Error)]
pub enum CampaignError {
    /// The request asks for a reduction with no evaluations to spend.
    #[error("reduction-budget must be positive")]
    ReductionBudget,
    /// The finding limit is outside what an engine retains.
    #[error("finding-limit must be within 1..={}", MAX_RETAINED_FINDINGS)]
    FindingLimit,
    /// The request gives a sequence length to an instruction engine.
    #[error("sequence-words applies only to sequence campaigns")]
    SequenceWords,
    /// The shard index is not below a positive shard count.
    #[error("shard must be below a positive shards count")]
    Shard,
    /// The artifacts directory is not valid UTF-8.
    #[error("artifacts-dir must be valid UTF-8")]
    ArtifactsDirEncoding,
    /// The artifacts directory is empty once its trailing separators go.
    #[error("artifacts-dir must not be empty")]
    ArtifactsDir,
    /// The range holds no case, or runs past the last case index.
    #[error("campaign range starting at {first} cannot hold {count} cases")]
    Range {
        /// First case index.
        first: u64,
        /// Case count.
        count: u64,
    },
    /// The early stop lies past the range.
    #[error("cancel-after exceeds the case count")]
    CancelAfter,
    /// The worker count is zero or past [`MAX_CAMPAIGN_WORKERS`].
    #[error("workers must be within 1..={}", MAX_CAMPAIGN_WORKERS)]
    Workers,
    /// Shards times workers overflows.
    #[error("shards times workers overflows")]
    ShardsTimesWorkers,
    /// The deadline is zero.
    #[error("deadline-ms must be positive")]
    Deadline,
    /// A worker's shard index overflows.
    #[error("worker shard index overflows")]
    WorkerShardOverflow,
    /// A worker's shard index falls outside the partition.
    #[error("worker shard index is outside the partition")]
    WorkerShardOutside,
    /// The engine refused the configuration.
    #[error("invalid campaign configuration: {0}")]
    Configuration(#[from] ConfigurationError),
    /// A report counter overflowed.
    #[error("report counters overflowed")]
    CounterOverflow,
    /// The host could not run a batch.
    #[error(transparent)]
    Worker(#[from] WorkerFailure),
}

impl CampaignError {
    /// Whether this refuses the request itself, rather than failing a run
    /// of it.
    pub const fn is_request_refusal(&self) -> bool {
        !matches!(self, Self::CounterOverflow | Self::Worker(_))
    }
}

/// Why the host could not run a batch.
#[derive(Debug, thiserror::Error)]
pub enum WorkerFailure {
    /// A worker panicked outside its target boundary.
    #[error("an engine worker panicked outside its target boundary")]
    Panicked,
    /// A worker could not start.
    #[error("start engine worker: {0}")]
    Spawn(#[source] std::io::Error),
}

/// A failure the run reports and continues past.
#[derive(Debug)]
pub enum CampaignFailure {
    /// A retained finding's reduction failed; its original case is kept.
    Reduction {
        /// Original case index.
        case_index: u64,
        /// Why the reduction failed.
        error: ReductionError,
    },
    /// A finding's artifact could not be built.
    ArtifactBuild(ArtifactError),
    /// The store refused a finding's artifact.
    ArtifactStore {
        /// Where the store refused it.
        path: PathBuf,
        /// The refusal.
        error: ArtifactStoreError,
        /// The evidence no file holds.
        artifact: Box<FuzzFindingArtifact>,
    },
}

/// What a campaign asks of the caller.
pub trait CampaignHost {
    /// Whether the caller's deadline has passed. The run asks before each
    /// batch.
    fn expired(&self) -> bool;

    /// Runs every configuration of one batch on `target`, returning the
    /// runs in the order given.
    ///
    /// # Errors
    ///
    /// [`WorkerFailure`] when a worker could not start or panicked.
    fn run_batch(
        &mut self,
        target: FuzzTarget,
        configs: Vec<FuzzConfig>,
    ) -> Result<Vec<FuzzRun>, WorkerFailure>;

    /// The case indices the run will consider, before the first batch.
    fn planned(&mut self, _cases: u64) {}

    /// A batch of `count` cases from `first` starts, after `findings`
    /// findings so far.
    fn batch_started(&mut self, _first: u64, _count: u64, _findings: u64) {}

    /// A batch of `count` cases finished.
    fn batch_finished(&mut self, _count: u64) {}

    /// A failure the run continues past.
    fn failed(&mut self, failure: CampaignFailure);
}

/// A campaign after every refusal check passed.
#[derive(Debug, Clone)]
pub struct CampaignPlan {
    request: CampaignRequest,
    /// First case index the schedule considers.
    first: u64,
    /// Case indices the schedule declares.
    count: u64,
    /// Case indices the loop considers: `count`, or the early stop.
    limit: u64,
    /// The artifacts directory without its trailing separators.
    artifacts_dir: String,
    workers_u32: u32,
    total_shards: u32,
    config: FuzzConfig,
}

impl CampaignRequest {
    /// Refuses or accepts this request before the run schedules a case.
    ///
    /// # Errors
    ///
    /// [`CampaignError`] for every request no run could serve.
    // [Manes2021 p:3 s:2.3 Fuzz Testing Algorithm] The model fuzzer runs one preprocessing step before its iteration loop. Every refusal below lands in that step, before the loop schedules the first case.
    pub fn plan(self) -> Result<CampaignPlan, CampaignError> {
        if self.reduction.is_some_and(|request| request.budget == 0) {
            return Err(CampaignError::ReductionBudget);
        }
        if self.finding_limit == 0 || self.finding_limit > MAX_RETAINED_FINDINGS {
            return Err(CampaignError::FindingLimit);
        }
        if !self.target.generates_sequences() && self.sequence_words.is_some() {
            return Err(CampaignError::SequenceWords);
        }
        if self.shards == 0 || self.shard >= self.shards {
            return Err(CampaignError::Shard);
        }
        let artifacts_dir = self
            .artifacts_dir
            .to_str()
            .ok_or(CampaignError::ArtifactsDirEncoding)?
            .trim_end_matches(['/', '\\'])
            .to_owned();
        if artifacts_dir.is_empty() {
            return Err(CampaignError::ArtifactsDir);
        }
        let (first, count) = self
            .replay_case
            .map_or((self.first, self.count), |index| (index, 1));
        if count == 0 || first.checked_add(count - 1).is_none() {
            return Err(CampaignError::Range { first, count });
        }
        let limit = self.cancel_after.unwrap_or(count);
        if limit > count {
            return Err(CampaignError::CancelAfter);
        }
        if self.workers == 0 || self.workers > MAX_CAMPAIGN_WORKERS {
            return Err(CampaignError::Workers);
        }
        let workers_u32 = u32::try_from(self.workers).map_err(|_| CampaignError::Workers)?;
        let total_shards = self
            .shards
            .checked_mul(workers_u32)
            .ok_or(CampaignError::ShardsTimesWorkers)?;
        if self.deadline_ms == Some(0) {
            return Err(CampaignError::Deadline);
        }
        let config = FuzzConfig {
            campaign_version: CampaignVersion(self.campaign_version),
            seed: self.seed,
            strategy: self.strategy,
            schedule: CampaignSchedule {
                cases: CaseRange { first, count },
                shard: CampaignShard {
                    index: self.shard,
                    count: self.shards,
                },
                cancellation: self.cancel_after.map(CancellationBoundary),
            },
            retention: Default::default(),
            max_findings: self.finding_limit,
            sequence_words: self.sequence_words.unwrap_or(DEFAULT_SEQUENCE_WORDS),
        };
        config.validate_for_target(self.target)?;
        Ok(CampaignPlan {
            request: self,
            first,
            count,
            limit,
            artifacts_dir,
            workers_u32,
            total_shards,
            config,
        })
    }
}

impl CampaignPlan {
    /// Case indices the loop considers: the range, or its early stop.
    pub const fn limit(&self) -> u64 {
        self.limit
    }

    /// Case indices the schedule declares.
    pub const fn count(&self) -> u64 {
        self.count
    }

    /// The engine the campaign runs.
    pub const fn target(&self) -> FuzzTarget {
        self.request.target
    }

    /// The whole campaign's engine configuration.
    pub const fn config(&self) -> FuzzConfig {
        self.config
    }

    /// This plan with `reference` as the independent source every
    /// artifact carries.
    #[must_use]
    pub fn with_reference(mut self, reference: ArtifactReference) -> Self {
        self.request.reference = reference;
        self
    }
}

/// One retained finding's artifact, as the summary names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRecord {
    /// Path the artifact write targeted.
    pub path: PathBuf,
    /// Generator version the finding replays under.
    pub campaign_version: u32,
    /// Master seed the finding replays under.
    pub seed: u64,
    /// Original case index.
    pub case_index: u64,
    /// The finding's kind.
    pub finding_kind: FindingKind,
    /// Stable fingerprint, as the artifact records it.
    pub fingerprint: ArtifactFingerprint,
    /// Reduction state stored with the finding.
    pub reduction: ArtifactReduction,
    /// Whether the artifact reached its path.
    pub stored: bool,
}

/// Counts of one generated campaign, accumulated over every worker run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CampaignSummary {
    /// Cases considered, decode refusals included.
    pub cases: u64,
    /// Decoded cases, or decoded instruction words for a sequence engine.
    pub decoded: u64,
    /// Cases eligible for their semantic check.
    pub eligible: u64,
    /// Cases the target refused as unmodeled.
    pub unsupported: u64,
    /// Cases the architecture leaves undefined.
    pub undefined: u64,
    /// Findings by kind, retained or not.
    pub finding_counts: BTreeMap<FindingKind, u64>,
    /// Retained findings, in storage order.
    pub artifacts: Vec<ArtifactRecord>,
    /// Retained findings whose reduction failed.
    pub reductions_failed: u64,
    /// Whether the range ended before the campaign considered every case
    /// index.
    pub cancelled: bool,
}

impl CampaignSummary {
    /// Findings that fail the campaign; see [`FindingKind::fails_the_run`].
    pub fn findings(&self) -> u64 {
        failing_findings(&self.finding_counts)
    }

    /// Adds one worker run's report to the totals.
    ///
    /// # Errors
    ///
    /// [`CampaignError::CounterOverflow`] when a total overflows.
    pub fn accumulate(&mut self, report: &FuzzReport) -> Result<(), CampaignError> {
        let add = |total: &mut u64, count: u64| {
            *total = total
                .checked_add(count)
                .ok_or(CampaignError::CounterOverflow)?;
            Ok::<(), CampaignError>(())
        };
        add(&mut self.cases, report.cases)?;
        add(&mut self.decoded, report.decoded)?;
        add(&mut self.eligible, report.eligible_cases)?;
        add(&mut self.unsupported, report.unsupported_cases)?;
        add(&mut self.undefined, report.undefined_cases)?;
        for (kind, count) in &report.finding_counts {
            add(self.finding_counts.entry(*kind).or_insert(0), *count)?;
        }
        Ok(())
    }
}

/// Terminal state of one generated campaign, in precedence order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CampaignOutcome {
    /// The store refused a finding's artifact.
    EvidenceNotStored,
    /// The engine failed inside the harness.
    HarnessFailure,
    /// A retained finding's reduction failed.
    ReductionFailed,
    /// The range ended before every case ran, findings or not.
    Cancelled,
    /// Every case ran and the campaign retained a finding or a target
    /// panic. The artifacts are the campaign's product, so the run
    /// succeeded.
    Findings,
    /// Every case ran and none was eligible.
    NoEligibleCases,
    /// Every case ran clean.
    Clean,
}

impl CampaignOutcome {
    /// Ranks what a campaign run met.
    pub fn classify(
        summary: &CampaignSummary,
        evidence_not_stored: bool,
        harness_failure: bool,
    ) -> Self {
        if evidence_not_stored {
            Self::EvidenceNotStored
        } else if harness_failure {
            Self::HarnessFailure
        } else if summary.reductions_failed > 0 {
            Self::ReductionFailed
        } else if summary.cancelled {
            Self::Cancelled
        } else if summary.findings() > 0 {
            Self::Findings
        } else if summary.eligible == 0 {
            Self::NoEligibleCases
        } else {
            Self::Clean
        }
    }
}

/// What a campaign run met.
#[derive(Debug, Default)]
pub struct CampaignRun {
    /// The accumulated counts and retained artifacts.
    pub summary: CampaignSummary,
    /// Case indices considered.
    pub offset: u64,
    /// The first engine failure inside the harness.
    pub harness_failure: Option<FuzzError>,
    /// Whether any finding's artifact went unstored.
    pub evidence_not_stored: bool,
}

impl CampaignRun {
    /// This run's terminal state.
    pub fn outcome(&self) -> CampaignOutcome {
        CampaignOutcome::classify(
            &self.summary,
            self.evidence_not_stored,
            self.harness_failure.is_some(),
        )
    }
}

/// Reduces one retained finding, keeping the refusal as its outcome.
pub fn reduce_retained_finding(
    config: FuzzConfig,
    finding: &Finding,
    request: ReductionRequest,
) -> ReductionOutcome {
    reduce_finding(config, finding, request)
        .map_or_else(ReductionOutcome::Failed, ReductionReport::into_outcome)
}

/// The shard one worker runs in a batch, rotated by `batch_shift` so
/// the workers of successive batches cover the selected shard between
/// them.
///
/// # Errors
///
/// [`CampaignError::WorkerShardOverflow`] or
/// [`CampaignError::WorkerShardOutside`] for an index outside the
/// partition.
pub(crate) fn worker_shard_index(
    shard: u32,
    shards: u32,
    worker: u32,
    total_shards: u32,
    batch_shift: u32,
) -> Result<u32, CampaignError> {
    let absolute = worker
        .checked_mul(shards)
        .and_then(|value| value.checked_add(shard))
        .ok_or(CampaignError::WorkerShardOverflow)?;
    if absolute >= total_shards || batch_shift >= total_shards {
        return Err(CampaignError::WorkerShardOutside);
    }
    Ok(if absolute >= batch_shift {
        absolute - batch_shift
    } else {
        total_shards - (batch_shift - absolute)
    })
}

/// Runs `plan` batch by batch through `host`.
///
/// The run reduces each retained finding when the plan asks for it,
/// then builds and stores its artifact. It reports a failure of either
/// to the host and continues; the run's outcome ranks the failure.
///
/// # Errors
///
/// [`CampaignError`] when the run cannot schedule a batch or a counter
/// overflows. The run stops there.
pub fn run_campaign(
    plan: &CampaignPlan,
    host: &mut dyn CampaignHost,
) -> Result<CampaignRun, CampaignError> {
    let mut run = CampaignRun::default();
    drive(plan, host, &mut run)?;
    run.summary.cancelled = run.offset < plan.count;
    Ok(run)
}

fn drive(
    plan: &CampaignPlan,
    host: &mut dyn CampaignHost,
    run: &mut CampaignRun,
) -> Result<(), CampaignError> {
    let request = &plan.request;
    let target = request.target;
    let mut artifact_index = 0u64;
    host.planned(plan.limit);
    // [Manes2021 p:3 s:2.3 Fuzz Testing Algorithm] The model loop iterates until its time limit passes or its continue check says stop; this loop keeps both exits.
    while run.offset < plan.limit {
        if host.expired() {
            break;
        }
        let batch = (plan.limit - run.offset).min(CAMPAIGN_BATCH_CASES);
        let batch_first = plan
            .first
            .checked_add(run.offset)
            .ok_or(CampaignError::Range {
                first: plan.first,
                count: plan.count,
            })?;
        host.batch_started(batch_first, batch, run.summary.findings());
        let shift = (run.offset % u64::from(plan.total_shards)) as u32;
        let mut configs = Vec::with_capacity(request.workers);
        for worker in 0..plan.workers_u32 {
            let local_shard = worker_shard_index(
                request.shard,
                request.shards,
                worker,
                plan.total_shards,
                shift,
            )?;
            configs.push(FuzzConfig {
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
                ..plan.config
            });
        }
        for worker_run in host.run_batch(target, configs)? {
            for finding in &worker_run.report.findings {
                store_finding(plan, host, run, &worker_run.report, finding, artifact_index)?;
                artifact_index = artifact_index
                    .checked_add(1)
                    .ok_or(CampaignError::CounterOverflow)?;
            }
            run.summary.accumulate(&worker_run.report)?;
            if let RunOutcome::HarnessFailure(source) = &worker_run.outcome {
                if run.harness_failure.is_none() {
                    run.harness_failure = Some(source.clone());
                }
            }
        }
        run.offset += batch;
        host.batch_finished(batch);
    }
    Ok(())
}

/// Reduces, builds and stores one retained finding, recording it in the
/// summary whether or not it reached its file.
fn store_finding(
    plan: &CampaignPlan,
    host: &mut dyn CampaignHost,
    run: &mut CampaignRun,
    report: &FuzzReport,
    finding: &Finding,
    artifact_index: u64,
) -> Result<(), CampaignError> {
    let request = &plan.request;
    let path = artifact_path(
        &plan.artifacts_dir,
        &format!(
            "{:?}-{:?}-{}",
            request.target, plan.config.strategy, request.seed
        ),
        finding.replay.case_index,
        artifact_index,
    );
    let mut finding = finding.clone();
    if let Some(reduction) = request.reduction {
        finding.reduction = reduce_retained_finding(plan.config, &finding, reduction);
        // The artifact records the refusal; the host names it too, like
        // the artifact failures below.
        if let ReductionOutcome::Failed(error) = &finding.reduction {
            run.summary.reductions_failed = run
                .summary
                .reductions_failed
                .checked_add(1)
                .ok_or(CampaignError::CounterOverflow)?;
            host.failed(CampaignFailure::Reduction {
                case_index: finding.replay.case_index,
                error: error.clone(),
            });
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
    let built = FuzzFindingArtifact::from_finding(
        plan.config,
        ArtifactExecutionPolicy {
            workers: plan.workers_u32,
            deadline_ms: request.deadline_ms,
            progress: request.progress,
            check: ArtifactCheckSelection::All,
            reduction: request
                .reduction
                .map_or(ArtifactReductionRequest::None, |reduction| {
                    ArtifactReductionRequest::OnFinding {
                        policy: reduction.policy,
                        budget: reduction.budget,
                    }
                }),
        },
        report,
        &finding,
        request.reference.clone(),
        &path,
    );
    // A failed write stops neither the findings after it nor the summary.
    // The host keeps every failure with its retained evidence.
    let failure = match built {
        Err(error) => Some(CampaignFailure::ArtifactBuild(error)),
        Ok(artifact) => match artifact.store(&path) {
            Ok(()) => None,
            Err(error) => Some(CampaignFailure::ArtifactStore {
                path: path.clone(),
                error,
                artifact: Box::new(artifact),
            }),
        },
    };
    match failure {
        None => record.stored = true,
        Some(failure) => {
            run.evidence_not_stored = true;
            host.failed(failure);
        }
    }
    run.summary.artifacts.push(record);
    Ok(())
}

#[cfg(test)]
#[path = "tests/runner_tests.rs"]
mod tests;
