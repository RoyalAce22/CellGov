//! `cellgov dev decoder-sweep` -- offline decoder artifacts.

use std::io::Write;
use std::path::PathBuf;

use cellgov_fuzz::raw_decode::{scan_raw_decoder, RawDecodeDomain, RawDecodeError, RawDecoder};

use super::exit::{CommandError, CommandExitCode};
use super::parse::{DecoderSweepArgs, SweepDecoder};

/// A sweep command failed before producing a clean artifact.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DecoderSweepError {
    /// Command flags do not select a bounded or full interval.
    #[error("decoder-sweep: use --full or --count; --start is bounded-only, --shard/--shards are full-only")]
    Selection,
    /// Domain or finite sweep refused an input.
    #[error("decoder-sweep: {0}")]
    Sweep(#[from] RawDecodeError),
    #[error("decoder-sweep: serialize result: {0}")]
    Json(#[from] serde_json::Error),
    #[error("decoder-sweep: write {}: {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("decoder-sweep: stdout write: {0}")]
    Stdout(#[source] std::io::Error),
}

impl DecoderSweepError {
    pub(crate) const fn is_usage(&self) -> bool {
        match self {
            Self::Selection => true,
            Self::Sweep(source) => source.is_invalid_request(),
            Self::Json(_) | Self::Write { .. } | Self::Stdout(_) => false,
        }
    }
}

pub(crate) fn run(args: &DecoderSweepArgs) -> Result<CommandExitCode, CommandError> {
    run_inner(args).map_err(CommandError::from)
}

#[cfg(test)]
#[path = "tests/decoder_sweep_tests.rs"]
mod tests;

fn run_inner(args: &DecoderSweepArgs) -> Result<CommandExitCode, DecoderSweepError> {
    let domain = if args.full {
        if args.start.is_some() {
            return Err(DecoderSweepError::Selection);
        }
        RawDecodeDomain::full_shard(args.shard.unwrap_or(0), args.shards.unwrap_or(1))?
    } else {
        if args.shard.is_some() || args.shards.is_some() {
            return Err(DecoderSweepError::Selection);
        }
        RawDecodeDomain::new(
            args.start.unwrap_or(0),
            args.count.ok_or(DecoderSweepError::Selection)?,
        )?
    };
    let decoder = match args.decoder {
        SweepDecoder::Ppu => RawDecoder::Ppu,
        SweepDecoder::Spu => RawDecoder::Spu,
    };
    let artifact = scan_raw_decoder(
        decoder,
        domain,
        args.chunk_size,
        args.workers,
        args.cancel_after,
    )?;
    let json = serde_json::to_vec_pretty(&artifact)?;
    std::fs::write(&args.output, json).map_err(|source| DecoderSweepError::Write {
        path: args.output.clone(),
        source,
    })?;
    let line = format!(
        "decoder-sweep: {:?} {} of {} words; accepted={} refused={} panics={} -> {}\n",
        artifact.status,
        artifact.processed,
        artifact.domain.count,
        artifact.accepted,
        artifact.refused,
        artifact.panics,
        args.output.display()
    );
    let stdout = write_report(&mut std::io::stdout().lock(), &line)?;
    if stdout != CommandExitCode::SUCCESS {
        return Ok(stdout);
    }
    Ok(if artifact.is_clean() {
        CommandExitCode::SUCCESS
    } else {
        CommandExitCode::new(super::exit_codes::FAILED)
    })
}

fn write_report(out: &mut impl Write, line: &str) -> Result<CommandExitCode, DecoderSweepError> {
    match out.write_all(line.as_bytes()).and_then(|()| out.flush()) {
        Ok(()) => Ok(CommandExitCode::SUCCESS),
        Err(source) if source.kind() == std::io::ErrorKind::BrokenPipe => {
            Ok(CommandExitCode::new(super::exit_codes::BROKEN_PIPE))
        }
        Err(source) => Err(DecoderSweepError::Stdout(source)),
    }
}
