//! Descriptor-derived semantic enumeration and raw decoder scans.

use std::io::Write;
use std::time::Instant;

use cellgov_fuzz::raw_decode::{
    scan_raw_decoder, RawDecodeArtifact, RawDecodeDomain, RawDecodeStatus, RawDecoder,
    MAX_RAW_DECODE_CHUNK, MAX_RAW_DECODE_PANIC_SAMPLES, RAW_DECODE_SCHEMA_VERSION,
};
use cellgov_fuzz::semantic_sweep::{sweep_ppu, sweep_spu, SemanticSweepReport};

use super::entry::{deadline, reports_progress, worker_count, write_stdout};
use super::error::FuzzCliError;
use super::outcome::{
    render_raw_progress, render_raw_summary, render_semantic_summary, RawSummary, SemanticSummary,
};
use crate::cli::exit::CommandExitCode;
use crate::cli::exit_codes;
use crate::cli::parse::{
    FuzzRawArgs, FuzzRawDecoder, FuzzReduction, FuzzSemanticArgs, FuzzSemanticTarget,
};

pub(super) fn run_semantic(
    args: &FuzzSemanticArgs,
    quiet: bool,
) -> Result<CommandExitCode, FuzzCliError> {
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
        write_stdout(&render_semantic_summary(&SemanticSummary {
            interpreter: name,
            kinds: report.expected_kinds.len(),
            witnesses: report.witnesses.len(),
            findings: report.findings.len(),
            refusals: report.expected_refusals,
        }))?;
    }
    Ok(if failed {
        CommandExitCode::new(exit_codes::FAILED)
    } else {
        CommandExitCode::SUCCESS
    })
}

pub(super) fn run_raw(args: &FuzzRawArgs, quiet: bool) -> Result<CommandExitCode, FuzzCliError> {
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
            eprintln!("{}", render_raw_progress(artifact.processed, domain.count));
        }
    }
    artifact.status = if artifact.processed == domain.count {
        RawDecodeStatus::Complete
    } else {
        RawDecodeStatus::Cancelled
    };
    let json = serde_json::to_vec_pretty(&artifact)?;
    let summary = RawSummary {
        decoder,
        status: artifact.status,
        processed: artifact.processed,
        domain: domain.count,
        accepted: artifact.accepted,
        refused: artifact.refused,
        panics: artifact.panics,
        output: args.output.clone(),
    };
    if let Some(path) = &args.output {
        std::fs::write(path, json).map_err(|source| FuzzCliError::Write {
            path: path.clone(),
            source,
        })?;
        write_stdout(&render_raw_summary(&summary))?;
    } else {
        let mut output = std::io::stdout().lock();
        output
            .write_all(&json)
            .and_then(|()| output.write_all(b"\n"))
            .map_err(FuzzCliError::Stdout)?;
    }
    Ok(summary.exit_code())
}
