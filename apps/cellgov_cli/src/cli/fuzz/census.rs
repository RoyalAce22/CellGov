//! The exhaustive PPU decoder census and the merge of its shards.

use std::io::Write;
use std::sync::mpsc;
use std::time::Instant;

use cellgov_fuzz::decode_census::{census, merge, DecodeCensusArtifact, DecodeCensusError};
use cellgov_fuzz::finite_partition_bounds;
use cellgov_fuzz::raw_decode::RawDecodeDomain;

use super::entry::{reports_progress, worker_count, write_stdout};
use super::error::FuzzCliError;
use super::outcome::{render_census_progress, render_census_summary, CensusSummary};
use crate::cli::exit::CommandExitCode;
use crate::cli::parse::{FuzzCensusArgs, FuzzCensusMergeArgs};

/// Words one worker classifies before it reports progress.
const PROGRESS_BATCH: u64 = 1 << 20;
/// Words in the whole 32-bit instruction space.
const FULL_SPACE_WORDS: u64 = 1 << 32;

pub(super) fn run_census(
    args: &FuzzCensusArgs,
    quiet: bool,
) -> Result<CommandExitCode, FuzzCliError> {
    let workers = worker_count(args.workers)?;
    // A refused interval is the census's own domain error, not a raw scan's:
    // the bare `?` would convert it through the `Raw` arm and report it as
    // "raw decoder scan".
    let domain = if args.full {
        if args.start.is_some() {
            return Err(FuzzCliError::Invalid("start is bounded-only"));
        }
        RawDecodeDomain::full_shard(args.shard.unwrap_or(0), args.shards.unwrap_or(1))
            .map_err(DecodeCensusError::Domain)?
    } else {
        if args.shard.is_some() || args.shards.is_some() {
            return Err(FuzzCliError::Invalid("shard is full-only"));
        }
        RawDecodeDomain::new(
            args.start.unwrap_or(0),
            args.count
                .ok_or(FuzzCliError::Invalid("count is required"))?,
        )
        .map_err(DecodeCensusError::Domain)?
    };
    let started = Instant::now();
    let progress = reports_progress(args.progress, quiet);
    let parts = classify_in_parallel(domain, workers, progress)?;
    let artifact = merge(&parts)?;
    finish(
        artifact,
        "fuzz census",
        started.elapsed().as_secs(),
        args.output.as_deref(),
    )
}

/// Splits the interval across `workers` threads and collects their parts.
///
/// The split assigns every word to a worker before any thread starts,
/// so the merged result does not depend on the worker count.
fn classify_in_parallel(
    domain: RawDecodeDomain,
    workers: usize,
    progress: bool,
) -> Result<Vec<DecodeCensusArtifact>, FuzzCliError> {
    let total = domain.count;
    // An interval shorter than the worker count leaves the surplus idle.
    let active_workers = usize::try_from(total.min(workers as u64)).unwrap_or(workers);
    let (sender, receiver) = mpsc::channel::<u64>();
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(active_workers);
        let mut spawn_error = None;
        for worker in 0..active_workers {
            let sender = sender.clone();
            let job = move || -> Result<Vec<DecodeCensusArtifact>, FuzzCliError> {
                let bounds = finite_partition_bounds(total as usize, active_workers, worker)
                    .map_err(FuzzCliError::CensusPartition)?;
                let mut parts = Vec::new();
                let mut offset = bounds.start as u64;
                while offset < bounds.end as u64 {
                    let count = (bounds.end as u64 - offset).min(PROGRESS_BATCH);
                    let first = u64::from(domain.first) + offset;
                    let first =
                        u32::try_from(first).map_err(|_| FuzzCliError::CensusOffsetOverflow)?;
                    parts.push(census(RawDecodeDomain::new(first, count)?)?);
                    offset += count;
                    sender
                        .send(count)
                        .map_err(|_| FuzzCliError::CensusProgressClosed)?;
                }
                Ok(parts)
            };
            match std::thread::Builder::new().spawn_scoped(scope, job) {
                Ok(handle) => handles.push(handle),
                Err(source) => {
                    spawn_error = Some(source);
                    break;
                }
            }
        }
        drop(sender);
        let mut done = 0u64;
        for count in receiver {
            done += count;
            if progress {
                eprintln!("{}", render_census_progress(done, total));
            }
        }
        // Every handle joins before any result is judged, so the scope
        // never exits with a running worker.
        let mut parts = Vec::new();
        let mut worker_error = None;
        let mut worker_panicked = false;
        for handle in handles {
            match handle.join() {
                Ok(Ok(worker_parts)) => parts.extend(worker_parts),
                Ok(Err(error)) => {
                    worker_error.get_or_insert(error);
                }
                Err(_) => worker_panicked = true,
            }
        }
        if worker_panicked {
            Err(FuzzCliError::WorkerPanic)
        } else if let Some(error) = worker_error {
            Err(error)
        } else if let Some(source) = spawn_error {
            Err(FuzzCliError::WorkerSpawn(source))
        } else {
            Ok(parts)
        }
    })
}

pub(super) fn run_census_merge(
    args: &FuzzCensusMergeArgs,
) -> Result<CommandExitCode, FuzzCliError> {
    let started = Instant::now();
    let mut parts = Vec::with_capacity(args.inputs.len());
    for path in &args.inputs {
        let json = std::fs::read_to_string(path).map_err(|source| FuzzCliError::CensusRead {
            path: path.clone(),
            source,
        })?;
        parts.push(DecodeCensusArtifact::parse_json(&json)?);
    }
    let artifact = merge(&parts)?;
    // A merge over files has no expected interval of its own: a lost
    // first or last shard tiles into a shorter interval that merge()
    // cannot see. The full-space request supplies the expectation.
    if args.full && (artifact.domain.first != 0 || artifact.domain.count != FULL_SPACE_WORDS) {
        return Err(FuzzCliError::CensusIncomplete {
            first: artifact.domain.first,
            count: artifact.domain.count,
        });
    }
    finish(
        artifact,
        "fuzz census-merge",
        started.elapsed().as_secs(),
        args.output.as_deref(),
    )
}

fn finish(
    artifact: DecodeCensusArtifact,
    command: &'static str,
    elapsed_seconds: u64,
    output: Option<&std::path::Path>,
) -> Result<CommandExitCode, FuzzCliError> {
    let summary = CensusSummary {
        command,
        words: artifact.domain.count,
        classes: artifact.classes,
        primary_zero_decoded: artifact.primary_zero_decoded(),
        findings: artifact.findings(),
        elapsed_seconds,
        output: output.map(std::path::Path::to_path_buf),
    };
    let json = serde_json::to_vec_pretty(&artifact)?;
    if let Some(path) = output {
        std::fs::write(path, json).map_err(|source| FuzzCliError::Write {
            path: path.to_path_buf(),
            source,
        })?;
        write_stdout(&render_census_summary(&summary))?;
    } else {
        let mut stdout = std::io::stdout().lock();
        stdout
            .write_all(&json)
            .and_then(|()| stdout.write_all(b"\n"))
            .map_err(FuzzCliError::Stdout)?;
    }
    Ok(summary.exit_code())
}
