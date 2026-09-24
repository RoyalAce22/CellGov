//! Descriptor-derived semantic enumeration and raw decoder scans.

use std::io::Write;
use std::time::Instant;

use cellgov_fuzz::raw_decode::{
    scan_raw_decoder_with, RawDecodeDomain, RawDecoder, RawScanHost, MAX_RAW_DECODE_CHUNK,
    MAX_RAW_DECODE_PANIC_SAMPLES,
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

/// The deadline and progress lines a raw scan runs under.
struct ScanHost {
    start: Instant,
    timeout: Option<std::time::Duration>,
    progress: bool,
    domain_count: u64,
}

impl RawScanHost for ScanHost {
    fn expired(&self) -> bool {
        self.timeout
            .is_some_and(|bound| self.start.elapsed() >= bound)
    }

    fn scanned(&mut self, processed: u64) {
        if self.progress {
            eprintln!("{}", render_raw_progress(processed, self.domain_count));
        }
    }
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
    let mut host = ScanHost {
        start: Instant::now(),
        timeout,
        progress: reports_progress(args.progress, quiet),
        domain_count: domain.count,
    };
    let artifact = scan_raw_decoder_with(
        decoder,
        domain,
        args.chunk_size,
        workers,
        args.cancel_after,
        &mut host,
    )?;
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
