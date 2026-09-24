//! Dispatch from the parsed `dev fuzz` subcommand to its runner, and the
//! host settings every runner shares.

use std::io::Write;
use std::time::Duration;

use cellgov_terminal::caps::RenderFlags;

use super::artifact::run_replay;
use super::campaign::run_campaign;
use super::census::{run_census, run_census_merge};
use super::error::FuzzCliError;
use super::evaluate::{run_compare, run_evaluate};
use super::scan::{run_raw, run_semantic};
use super::smoke::{run_promote, run_smoke};
use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::parse::{FuzzArgs, FuzzCommand};

pub(super) const MAX_HOST_WORKERS: usize = 64;

pub(crate) fn run_with_render(
    args: &FuzzArgs,
    render: RenderFlags,
) -> Result<CommandExitCode, CommandError> {
    run_inner_with_render(args, render).map_err(CommandError::from)
}

/// The render decision the in-process tests run under: no bar, so a
/// test never starts the one live render thread a process may own.
#[cfg(test)]
pub(super) const TEST_RENDER: RenderFlags = RenderFlags {
    no_progress: true,
    no_color: false,
    quiet: false,
    json: false,
    force_ansi: false,
};

#[cfg(test)]
pub(crate) fn run(args: &FuzzArgs) -> Result<CommandExitCode, CommandError> {
    run_with_render(args, TEST_RENDER)
}

#[cfg(test)]
pub(super) fn run_inner(args: &FuzzArgs) -> Result<CommandExitCode, FuzzCliError> {
    run_inner_with_render(args, TEST_RENDER)
}

pub(super) fn run_inner_with_render(
    args: &FuzzArgs,
    render: RenderFlags,
) -> Result<CommandExitCode, FuzzCliError> {
    let quiet = render.quiet;
    match &args.command {
        FuzzCommand::PpuInstruction(args) => {
            run_campaign(args, cellgov_fuzz::FuzzTarget::PpuInstruction, render)
        }
        FuzzCommand::PpuSequence(args) => {
            run_campaign(args, cellgov_fuzz::FuzzTarget::PpuSequence, render)
        }
        FuzzCommand::SpuInstruction(args) => {
            run_campaign(args, cellgov_fuzz::FuzzTarget::SpuInstruction, render)
        }
        FuzzCommand::SpuSequence(args) => {
            run_campaign(args, cellgov_fuzz::FuzzTarget::SpuSequence, render)
        }
        FuzzCommand::Semantic(args) => run_semantic(args, quiet),
        FuzzCommand::Raw(args) => run_raw(args, quiet),
        FuzzCommand::Census(args) => run_census(args, quiet),
        FuzzCommand::CensusMerge(args) => run_census_merge(args),
        FuzzCommand::Replay(args) => run_replay(args),
        FuzzCommand::Evaluate(args) => run_evaluate(args, quiet),
        FuzzCommand::Compare(args) => run_compare(args),
        FuzzCommand::Smoke(args) => run_smoke(args, quiet),
        FuzzCommand::Promote(args) => run_promote(args),
    }
}

pub(super) const fn reports_progress(requested: bool, quiet: bool) -> bool {
    requested && !quiet
}

pub(super) fn worker_count(requested: Option<usize>) -> Result<usize, FuzzCliError> {
    let available = std::thread::available_parallelism().map_or(1, |count| count.get());
    let workers = requested.unwrap_or(available.min(MAX_HOST_WORKERS));
    if workers == 0 || workers > MAX_HOST_WORKERS {
        return Err(FuzzCliError::Invalid("workers must be within 1..=64"));
    }
    Ok(workers)
}

pub(super) fn deadline(ms: Option<u64>) -> Result<Option<Duration>, FuzzCliError> {
    match ms {
        None => Ok(None),
        Some(0) => Err(FuzzCliError::Invalid("deadline-ms must be positive")),
        Some(value) => Ok(Some(Duration::from_millis(value))),
    }
}

pub(super) fn write_stdout(line: &str) -> Result<(), FuzzCliError> {
    std::io::stdout()
        .lock()
        .write_all(line.as_bytes())
        .map_err(FuzzCliError::Stdout)
}
