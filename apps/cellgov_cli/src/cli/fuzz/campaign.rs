//! Generated interpreter campaigns: worker scheduling, batch deadlines,
//! finding reduction, and artifact storage.

use std::time::Instant;

use cellgov_fuzz::artifact::{
    ArtifactCheckSelection, ArtifactExecutionPolicy, ArtifactFingerprint, ArtifactReduction,
    ArtifactReductionRequest, ArtifactReference, FuzzFindingArtifact,
};
use cellgov_fuzz::reduce::{reduce_finding, ReductionPolicy, ReductionReport, ReductionRequest};
use cellgov_fuzz::report::Finding;
use cellgov_fuzz::{
    ppu, spu, CampaignSchedule, CampaignShard, CampaignVersion, CaseRange, FuzzConfig, FuzzReport,
    FuzzRun, FuzzTarget, GenerationStrategy, ReductionOutcome, RunOutcome,
};

use super::artifact::{check_reference, persist_finding};
use super::entry::{deadline, reports_progress, worker_count, write_stdout};
use super::error::FuzzCliError;
use super::outcome::{
    render_campaign_progress, render_campaign_summary, ArtifactRecord, CampaignOutcome,
    CampaignProgress, CampaignSummary,
};
use crate::cli::exit::CommandExitCode;
use crate::cli::parse::{
    FuzzCampaignArgs, FuzzCheck, FuzzReduction, FuzzReductionPolicy, FuzzStrategy,
};

const CAMPAIGN_BATCH_CASES: u64 = 64;

/// One of the four generated-campaign engines.
#[derive(Debug, Clone, Copy)]
pub(super) enum FuzzEngine {
    PpuInstruction,
    PpuSequence,
    SpuInstruction,
    SpuSequence,
}

impl FuzzEngine {
    fn run(self, config: FuzzConfig) -> FuzzRun {
        match self {
            Self::PpuInstruction => ppu::run_instructions(config),
            Self::PpuSequence => ppu::run_sequences(config),
            Self::SpuInstruction => spu::run_instructions(config),
            Self::SpuSequence => spu::run_sequences(config),
        }
    }

    pub(super) const fn is_ppu(self) -> bool {
        matches!(self, Self::PpuInstruction | Self::PpuSequence)
    }

    pub(super) const fn target(self) -> FuzzTarget {
        match self {
            Self::PpuInstruction => FuzzTarget::PpuInstruction,
            Self::PpuSequence => FuzzTarget::PpuSequence,
            Self::SpuInstruction => FuzzTarget::SpuInstruction,
            Self::SpuSequence => FuzzTarget::SpuSequence,
        }
    }
}

pub(super) fn run_campaign(
    args: &FuzzCampaignArgs,
    engine: FuzzEngine,
    quiet: bool,
) -> Result<CommandExitCode, FuzzCliError> {
    // [Manes2021 p:1 s:Abstract] A model fuzzer has distinct stages with separate design decisions.
    if !matches!(args.check, FuzzCheck::All) {
        return Err(FuzzCliError::CheckUnavailable);
    }
    let reduction = match args.reduction {
        FuzzReduction::None => None,
        FuzzReduction::OnFinding => Some(ReductionRequest {
            policy: match args.reduction_policy {
                FuzzReductionPolicy::Deterministic => ReductionPolicy::Deterministic,
                FuzzReductionPolicy::Greedy => ReductionPolicy::Greedy,
            },
            budget: args.reduction_budget,
        }),
    };
    if reduction.is_some_and(|request| request.budget == 0) {
        return Err(FuzzCliError::Invalid("reduction-budget must be positive"));
    }
    if args.finding_limit == 0 || args.finding_limit > 1_024 {
        return Err(FuzzCliError::Invalid(
            "finding-limit must be within 1..=1024",
        ));
    }
    if matches!(
        engine,
        FuzzEngine::PpuInstruction | FuzzEngine::SpuInstruction
    ) && args.sequence_words.is_some()
    {
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
        strategy: match args.strategy {
            FuzzStrategy::Structured => GenerationStrategy::Structured,
            FuzzStrategy::RawWords => GenerationStrategy::RawWords,
        },
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
        sequence_words: args.sequence_words.unwrap_or(32),
    };
    campaign_config.validate_for_target(engine.target())?;
    let start = Instant::now();
    let reference = if let Some(path) = &args.reference {
        check_reference(path, engine)?
    } else {
        ArtifactReference::Local
    };
    let mut offset = 0u64;
    let mut summary = CampaignSummary::default();
    let mut harness_failure = None;
    let mut artifact_index = 0u64;
    let mut artifact_failure = None;
    while offset < limit {
        if timeout.is_some_and(|bound| start.elapsed() >= bound) {
            break;
        }
        let batch = (limit - offset).min(CAMPAIGN_BATCH_CASES);
        let batch_first = first
            .checked_add(offset)
            .ok_or(FuzzCliError::Range { first, count })?;
        let shift = (offset % u64::from(total_shards)) as u32;
        let mut work = Vec::with_capacity(workers);
        for worker in 0..workers_u32 {
            let local_shard =
                worker_shard_index(args.shard, args.shards, worker, total_shards, shift)?;
            let config = FuzzConfig {
                campaign_version: CampaignVersion(args.campaign_version),
                seed: args.seed,
                strategy: match args.strategy {
                    FuzzStrategy::Structured => GenerationStrategy::Structured,
                    FuzzStrategy::RawWords => GenerationStrategy::RawWords,
                },
                schedule: CampaignSchedule {
                    cases: CaseRange {
                        first: batch_first,
                        count: batch,
                    },
                    shard: CampaignShard {
                        index: local_shard,
                        count: total_shards,
                    },
                    cancellation: None,
                },
                retention: Default::default(),
                max_findings: args.finding_limit,
                sequence_words: args.sequence_words.unwrap_or(32),
            };
            work.push(move || engine.run(config));
        }
        let runs = run_workers(work)?;
        for run in runs {
            for finding in &run.report.findings {
                // Portable text, spelled as `regression::promote` spells a
                // stored replay path: the directory as the caller gave it,
                // one forward slash, then the file.
                let path = std::path::PathBuf::from(format!(
                    "{}/{:?}-{:?}-{}-{}-{}.json",
                    artifacts_dir,
                    engine.target(),
                    campaign_config.strategy,
                    args.seed,
                    finding.replay.case_index,
                    artifact_index,
                ));
                let mut finding = finding.clone();
                if let Some(request) = reduction {
                    finding.reduction = reduce_retained_finding(campaign_config, &finding, request);
                    // The artifact records the refusal; the terminal names it too,
                    // like the artifact write failures below.
                    if let ReductionOutcome::Failed(error) = &finding.reduction {
                        summary.reductions_failed = summary
                            .reductions_failed
                            .checked_add(1)
                            .ok_or(FuzzCliError::CounterOverflow)?;
                        eprintln!(
                            "fuzz: reduction of case {} failed: {error}; original case kept",
                            finding.replay.case_index
                        );
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
                    campaign_config,
                    ArtifactExecutionPolicy {
                        workers: workers_u32,
                        deadline_ms: args.deadline_ms,
                        progress: args.progress,
                        check: ArtifactCheckSelection::All,
                        reduction: reduction.map_or(ArtifactReductionRequest::None, |request| {
                            ArtifactReductionRequest::OnFinding {
                                policy: request.policy,
                                budget: request.budget,
                            }
                        }),
                    },
                    &run.report,
                    &finding,
                    reference.clone(),
                    &path,
                )
                .map_err(FuzzCliError::from)
                .and_then(|artifact| persist_finding(&path, artifact));
                // A failed write stops neither the findings after it nor the
                // summary line. The loop prints every failure with its retained
                // evidence, and the first failure becomes the command's result.
                match stored {
                    Ok(()) => record.stored = true,
                    Err(error) => {
                        eprintln!("fuzz: {error}");
                        if artifact_failure.is_none() {
                            artifact_failure = Some(error);
                        }
                    }
                }
                summary.artifacts.push(record);
                artifact_index = artifact_index
                    .checked_add(1)
                    .ok_or(FuzzCliError::CounterOverflow)?;
            }
            accumulate(&mut summary, &run.report)?;
            if let RunOutcome::HarnessFailure(source) = &run.outcome {
                if harness_failure.is_none() {
                    harness_failure = Some(source.clone());
                }
            }
        }
        offset += batch;
        if reports_progress(args.progress, quiet) {
            eprintln!(
                "{}",
                render_campaign_progress(CampaignProgress {
                    considered: offset,
                    count,
                    processed: summary.cases,
                })
            );
        }
    }
    summary.cancelled = offset < count;
    let outcome = CampaignOutcome::classify(
        &summary,
        artifact_failure.is_some(),
        harness_failure.is_some(),
    );
    write_stdout(&render_campaign_summary(engine.target(), &summary, outcome))?;
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
