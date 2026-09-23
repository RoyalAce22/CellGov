//! Host policy for `cellgov dev fuzz` campaigns.

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use cellgov_fuzz::raw_decode::{
    scan_raw_decoder, RawDecodeArtifact, RawDecodeDomain, RawDecodeError, RawDecodeStatus,
    RawDecoder, MAX_RAW_DECODE_CHUNK, MAX_RAW_DECODE_PANIC_SAMPLES, RAW_DECODE_SCHEMA_VERSION,
};
use cellgov_fuzz::semantic_sweep::{sweep_ppu, sweep_spu, SemanticSweepReport};
use cellgov_fuzz::{
    ppu, spu, CampaignSchedule, CampaignShard, CampaignVersion, CaseRange, FuzzConfig, FuzzRun,
    FuzzTarget, GenerationStrategy, RunOutcome,
};

use super::exit::{CommandError, CommandExitCode};
use super::parse::{
    FuzzArgs, FuzzCampaignArgs, FuzzCheck, FuzzCommand, FuzzRawArgs, FuzzRawDecoder, FuzzReduction,
    FuzzSemanticArgs, FuzzSemanticTarget, FuzzStrategy,
};

const CAMPAIGN_BATCH_CASES: u64 = 64;
const MAX_HOST_WORKERS: usize = 64;

#[derive(Debug, thiserror::Error)]
pub(crate) enum FuzzCliError {
    #[error("fuzz: {0}")]
    Invalid(&'static str),
    #[error("fuzz: campaign range starting at {first} cannot hold {count} cases")]
    Range { first: u64, count: u64 },
    #[error("fuzz: selected check is not independently switchable by this engine")]
    CheckUnavailable,
    #[error("fuzz: requested reduction is unavailable until finding replay is installed")]
    ReductionUnavailable,
    #[error("fuzz: invalid campaign configuration: {0}")]
    Configuration(#[from] cellgov_fuzz::ConfigurationError),
    #[error("fuzz: raw scans retain exactly {MAX_RAW_DECODE_PANIC_SAMPLES} panic samples")]
    RawFindingLimit,
    #[error("fuzz: reference read {}: {source}", path.display())]
    ReferenceRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("fuzz: PPU reference: {0}")]
    PpuReference(#[from] cellgov_fuzz::ppu_reference::PpuReferenceError),
    #[error("fuzz: SPU reference: {0}")]
    SpuReference(#[from] cellgov_fuzz::spu_reference::SpuReferenceError),
    #[error("fuzz: independent reference differs from the interpreter")]
    ReferenceMismatch,
    #[error("fuzz: raw decoder scan: {0}")]
    Raw(#[from] RawDecodeError),
    #[error("fuzz: JSON serialization: {0}")]
    Json(#[from] serde_json::Error),
    #[error("fuzz: write {}: {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("fuzz: stdout write: {0}")]
    Stdout(#[source] std::io::Error),
    #[error("fuzz: an engine worker panicked outside its target boundary")]
    WorkerPanic,
    #[error("fuzz: start engine worker: {0}")]
    WorkerSpawn(#[source] std::io::Error),
    #[error("fuzz: engine failed: {0}")]
    Harness(#[source] cellgov_fuzz::FuzzError),
    #[error("fuzz: report counters overflowed")]
    CounterOverflow,
    #[error(
        "fuzz: campaign selected no eligible cases (cases={cases}, decoded={decoded}, unsupported={unsupported}, undefined={undefined})"
    )]
    NoEligibleCases {
        cases: u64,
        decoded: u64,
        unsupported: u64,
        undefined: u64,
    },
}

impl FuzzCliError {
    pub(crate) const fn is_usage(&self) -> bool {
        match self {
            Self::Invalid(_)
            | Self::Range { .. }
            | Self::CheckUnavailable
            | Self::ReductionUnavailable
            | Self::Configuration(_)
            | Self::RawFindingLimit
            | Self::Harness(cellgov_fuzz::FuzzError::Configuration(_)) => true,
            Self::Raw(source) => source.is_invalid_request(),
            Self::ReferenceRead { .. }
            | Self::PpuReference(_)
            | Self::SpuReference(_)
            | Self::ReferenceMismatch
            | Self::Json(_)
            | Self::Write { .. }
            | Self::Stdout(_)
            | Self::WorkerPanic
            | Self::WorkerSpawn(_)
            | Self::Harness(_)
            | Self::CounterOverflow
            | Self::NoEligibleCases { .. } => false,
        }
    }

    pub(crate) fn is_broken_pipe(&self) -> bool {
        matches!(self, Self::Stdout(source) if source.kind() == std::io::ErrorKind::BrokenPipe)
    }
}

#[derive(Debug, Clone, Copy)]
enum FuzzEngine {
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

    const fn is_ppu(self) -> bool {
        matches!(self, Self::PpuInstruction | Self::PpuSequence)
    }

    const fn target(self) -> FuzzTarget {
        match self {
            Self::PpuInstruction => FuzzTarget::PpuInstruction,
            Self::PpuSequence => FuzzTarget::PpuSequence,
            Self::SpuInstruction => FuzzTarget::SpuInstruction,
            Self::SpuSequence => FuzzTarget::SpuSequence,
        }
    }
}

pub(crate) fn run_with_quiet(
    args: &FuzzArgs,
    quiet: bool,
) -> Result<CommandExitCode, CommandError> {
    run_inner_with_quiet(args, quiet).map_err(CommandError::from)
}

#[cfg(test)]
pub(crate) fn run(args: &FuzzArgs) -> Result<CommandExitCode, CommandError> {
    run_with_quiet(args, false)
}

#[cfg(test)]
fn run_inner(args: &FuzzArgs) -> Result<CommandExitCode, FuzzCliError> {
    run_inner_with_quiet(args, false)
}

fn run_inner_with_quiet(args: &FuzzArgs, quiet: bool) -> Result<CommandExitCode, FuzzCliError> {
    match &args.command {
        FuzzCommand::PpuInstruction(args) => run_campaign(args, FuzzEngine::PpuInstruction, quiet),
        FuzzCommand::PpuSequence(args) => run_campaign(args, FuzzEngine::PpuSequence, quiet),
        FuzzCommand::SpuInstruction(args) => run_campaign(args, FuzzEngine::SpuInstruction, quiet),
        FuzzCommand::SpuSequence(args) => run_campaign(args, FuzzEngine::SpuSequence, quiet),
        FuzzCommand::Semantic(args) => run_semantic(args, quiet),
        FuzzCommand::Raw(args) => run_raw(args, quiet),
    }
}

const fn reports_progress(requested: bool, quiet: bool) -> bool {
    requested && !quiet
}

fn worker_count(requested: Option<usize>) -> Result<usize, FuzzCliError> {
    let available = std::thread::available_parallelism().map_or(1, |count| count.get());
    let workers = requested.unwrap_or(available.min(MAX_HOST_WORKERS));
    if workers == 0 || workers > MAX_HOST_WORKERS {
        return Err(FuzzCliError::Invalid("workers must be within 1..=64"));
    }
    Ok(workers)
}

fn deadline(ms: Option<u64>) -> Result<Option<Duration>, FuzzCliError> {
    match ms {
        None => Ok(None),
        Some(0) => Err(FuzzCliError::Invalid("deadline-ms must be positive")),
        Some(value) => Ok(Some(Duration::from_millis(value))),
    }
}

fn run_campaign(
    args: &FuzzCampaignArgs,
    engine: FuzzEngine,
    quiet: bool,
) -> Result<CommandExitCode, FuzzCliError> {
    // [Manes2021 p:1 s:Abstract] A model fuzzer has distinct stages with separate design decisions.
    if !matches!(args.check, FuzzCheck::All) {
        return Err(FuzzCliError::CheckUnavailable);
    }
    if !matches!(args.reduction, FuzzReduction::None) {
        return Err(FuzzCliError::ReductionUnavailable);
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
    FuzzConfig {
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
    }
    .validate_for_target(engine.target())?;
    let start = Instant::now();
    if let Some(path) = &args.reference {
        check_reference(path, engine)?;
    }
    let mut offset = 0u64;
    let mut cases = 0u64;
    let mut decoded = 0u64;
    let mut eligible = 0u64;
    let mut unsupported = 0u64;
    let mut undefined = 0u64;
    let mut findings = 0u64;
    let mut failed = false;
    let mut harness_failure = None;
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
            cases = cases
                .checked_add(run.report.cases)
                .ok_or(FuzzCliError::CounterOverflow)?;
            decoded = decoded
                .checked_add(run.report.decoded)
                .ok_or(FuzzCliError::CounterOverflow)?;
            eligible = eligible
                .checked_add(run.report.eligible_cases)
                .ok_or(FuzzCliError::CounterOverflow)?;
            unsupported = unsupported
                .checked_add(run.report.unsupported_cases)
                .ok_or(FuzzCliError::CounterOverflow)?;
            undefined = undefined
                .checked_add(run.report.undefined_cases)
                .ok_or(FuzzCliError::CounterOverflow)?;
            let run_findings = run
                .report
                .finding_counts
                .values()
                .try_fold(0u64, |sum, &count| {
                    sum.checked_add(count).ok_or(FuzzCliError::CounterOverflow)
                })?;
            findings = findings
                .checked_add(run_findings)
                .ok_or(FuzzCliError::CounterOverflow)?;
            if let RunOutcome::HarnessFailure(source) = &run.outcome {
                if harness_failure.is_none() {
                    harness_failure = Some(source.clone());
                }
            }
            failed |= matches!(
                run.outcome,
                RunOutcome::SemanticFinding
                    | RunOutcome::TargetPanic
                    | RunOutcome::HarnessFailure(_)
            );
        }
        offset += batch;
        if reports_progress(args.progress, quiet) {
            eprintln!("fuzz: considered {offset} of {count} case indices; processed {cases}");
        }
    }
    let cancelled = offset < count;
    let line = format!(
        "fuzz: {:?} cases={cases} decoded={decoded} eligible={eligible} unsupported={unsupported} undefined={undefined} findings={findings} cancelled={cancelled}\n",
        engine
    );
    write_stdout(&line)?;
    if let Some(source) = harness_failure {
        return Err(FuzzCliError::Harness(source));
    }
    if !cancelled && eligible == 0 && !failed {
        return Err(FuzzCliError::NoEligibleCases {
            cases,
            decoded,
            unsupported,
            undefined,
        });
    }
    Ok(if failed || cancelled {
        CommandExitCode::new(super::exit_codes::FAILED)
    } else {
        CommandExitCode::SUCCESS
    })
}

fn run_workers<T: Send, F: FnOnce() -> T + Send>(work: Vec<F>) -> Result<Vec<T>, FuzzCliError> {
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

fn worker_shard_index(
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

fn check_reference(path: &PathBuf, engine: FuzzEngine) -> Result<(), FuzzCliError> {
    let json = std::fs::read_to_string(path).map_err(|source| FuzzCliError::ReferenceRead {
        path: path.clone(),
        source,
    })?;
    if engine.is_ppu() {
        let artifact = cellgov_fuzz::ppu_reference::parse_reference_json(&json)?;
        let replay = cellgov_fuzz::ppu_reference::replay_reference(&artifact)?;
        if replay.comparisons.is_empty()
            || replay.internal_divergence.is_some()
            || replay
                .comparisons
                .iter()
                .any(|comparison| !comparison.is_match())
        {
            return Err(FuzzCliError::ReferenceMismatch);
        }
    } else {
        let artifact = cellgov_fuzz::spu_reference::parse_reference_json(&json)?;
        let replay = cellgov_fuzz::spu_reference::replay_reference(&artifact)?;
        if !replay.comparison.is_match() {
            return Err(FuzzCliError::ReferenceMismatch);
        }
    }
    Ok(())
}

fn run_semantic(args: &FuzzSemanticArgs, quiet: bool) -> Result<CommandExitCode, FuzzCliError> {
    let mut reports = Vec::<(&'static str, SemanticSweepReport)>::new();
    if matches!(
        args.target,
        FuzzSemanticTarget::Both | FuzzSemanticTarget::Ppu
    ) {
        reports.push((
            "ppu",
            sweep_ppu(&cellgov_ppu::instruction::fuzz::generation_descriptors()),
        ));
        if reports_progress(args.progress, quiet) {
            eprintln!("fuzz: enumerated PPU descriptors");
        }
    }
    if matches!(
        args.target,
        FuzzSemanticTarget::Both | FuzzSemanticTarget::Spu
    ) {
        reports.push((
            "spu",
            sweep_spu(&cellgov_spu::fuzz::generation_descriptors()),
        ));
        if reports_progress(args.progress, quiet) {
            eprintln!("fuzz: enumerated SPU descriptors");
        }
    }
    let mut failed = false;
    for (name, report) in reports {
        failed |= !report.is_clean();
        write_stdout(&format!(
            "fuzz semantic {name}: kinds={} witnesses={} findings={} refusals={}\n",
            report.expected_kinds.len(),
            report.witnesses.len(),
            report.findings.len(),
            report.expected_refusals
        ))?;
    }
    Ok(if failed {
        CommandExitCode::new(super::exit_codes::FAILED)
    } else {
        CommandExitCode::SUCCESS
    })
}

fn run_raw(args: &FuzzRawArgs, quiet: bool) -> Result<CommandExitCode, FuzzCliError> {
    if !matches!(args.reduction, FuzzReduction::None) {
        return Err(FuzzCliError::ReductionUnavailable);
    }
    if args.finding_limit != MAX_RAW_DECODE_PANIC_SAMPLES {
        return Err(FuzzCliError::RawFindingLimit);
    }
    if args.chunk_size == 0 || args.chunk_size > MAX_RAW_DECODE_CHUNK {
        return Err(FuzzCliError::Invalid(
            "chunk-size is outside the bounded scan limit",
        ));
    }
    let workers = worker_count(args.workers)?;
    let timeout = deadline(args.deadline_ms)?;
    let decoder = match args.decoder {
        FuzzRawDecoder::Ppu => RawDecoder::Ppu,
        FuzzRawDecoder::Spu => RawDecoder::Spu,
    };
    let domain = if args.full {
        if args.start.is_some() {
            return Err(FuzzCliError::Invalid("start is bounded-only"));
        }
        RawDecodeDomain::full_shard(args.shard.unwrap_or(0), args.shards.unwrap_or(1))?
    } else {
        if args.shard.is_some() || args.shards.is_some() {
            return Err(FuzzCliError::Invalid("shard is full-only"));
        }
        RawDecodeDomain::new(
            args.start.unwrap_or(0),
            args.count
                .ok_or(FuzzCliError::Invalid("count is required"))?,
        )?
    };
    let limit = args.cancel_after.unwrap_or(domain.count);
    if limit > domain.count {
        return Err(FuzzCliError::Invalid(
            "cancel-after exceeds the selected range",
        ));
    }
    let start = Instant::now();
    let mut artifact = RawDecodeArtifact {
        schema_version: RAW_DECODE_SCHEMA_VERSION,
        decoder,
        domain,
        status: RawDecodeStatus::Cancelled,
        processed: 0,
        accepted: 0,
        refused: 0,
        panics: 0,
        panic_samples: Vec::new(),
    };
    while artifact.processed < limit {
        if timeout.is_some_and(|bound| start.elapsed() >= bound) {
            break;
        }
        let count = (limit - artifact.processed).min(args.chunk_size as u64);
        let first = u64::from(domain.first) + artifact.processed;
        let first =
            u32::try_from(first).map_err(|_| FuzzCliError::Invalid("raw scan offset overflows"))?;
        let part = scan_raw_decoder(
            decoder,
            RawDecodeDomain::new(first, count)?,
            args.chunk_size,
            workers,
            None,
        )?;
        artifact.accepted += part.accepted;
        artifact.refused += part.refused;
        artifact.panics += part.panics;
        artifact.processed += part.processed;
        let available = MAX_RAW_DECODE_PANIC_SAMPLES.saturating_sub(artifact.panic_samples.len());
        artifact
            .panic_samples
            .extend(part.panic_samples.into_iter().take(available));
        if reports_progress(args.progress, quiet) {
            eprintln!("fuzz raw: {} of {} words", artifact.processed, domain.count);
        }
    }
    artifact.status = if artifact.processed == domain.count {
        RawDecodeStatus::Complete
    } else {
        RawDecodeStatus::Cancelled
    };
    let json = serde_json::to_vec_pretty(&artifact)?;
    if let Some(path) = &args.output {
        std::fs::write(path, json).map_err(|source| FuzzCliError::Write {
            path: path.clone(),
            source,
        })?;
        write_stdout(&format!(
            "fuzz raw: {:?} {} of {} words; accepted={} refused={} panics={} -> {}\n",
            artifact.status,
            artifact.processed,
            domain.count,
            artifact.accepted,
            artifact.refused,
            artifact.panics,
            path.display()
        ))?;
    } else {
        let mut output = std::io::stdout().lock();
        output
            .write_all(&json)
            .and_then(|()| output.write_all(b"\n"))
            .map_err(FuzzCliError::Stdout)?;
    }
    Ok(if artifact.is_clean() {
        CommandExitCode::SUCCESS
    } else {
        CommandExitCode::new(super::exit_codes::FAILED)
    })
}

fn write_stdout(line: &str) -> Result<(), FuzzCliError> {
    std::io::stdout()
        .lock()
        .write_all(line.as_bytes())
        .map_err(FuzzCliError::Stdout)
}

#[cfg(test)]
#[path = "tests/fuzz_tests.rs"]
mod tests;
