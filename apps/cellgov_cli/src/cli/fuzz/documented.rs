//! `dev fuzz spu-documented`: generate the SPU floating-point list the
//! documents define, replay it, and optionally write it.

use std::fmt::Write as _;

use cellgov_fuzz::spu_documented::{
    documented_vectors, render, replay_documented, DocumentedReplay,
};

use super::entry::write_stdout;
use super::error::FuzzCliError;
use crate::cli::exit::CommandExitCode;
use crate::cli::exit_codes;
use crate::cli::parse::FuzzSpuDocumentedArgs;

/// Writes the list when `--output` names a path, replays it, and prints the report.
///
/// The exit code is `FAILED` when a component differs or a vector stops.
pub(super) fn run_spu_documented(
    args: &FuzzSpuDocumentedArgs,
) -> Result<CommandExitCode, FuzzCliError> {
    let vectors = documented_vectors();
    if let Some(path) = &args.output {
        std::fs::write(path, render(&vectors)?).map_err(|source| FuzzCliError::Write {
            path: path.clone(),
            source,
        })?;
    }
    let replay = replay_documented(&vectors);
    write_stdout(&render_spu_documented(&replay))?;
    Ok(if replay.is_clean() {
        CommandExitCode::SUCCESS
    } else {
        CommandExitCode::new(exit_codes::FAILED)
    })
}

/// The report: one line per difference, one per stopped vector, then the summary.
///
/// The summary carries the undefined count of each question.
pub(super) fn render_spu_documented(replay: &DocumentedReplay) -> String {
    let mut text = String::new();
    for d in &replay.differences {
        let _ = writeln!(
            text,
            "fuzz spu-documented: {} {} differs mask={:032x} expected={:032x} observed={:032x}",
            d.vector, d.component, d.mask, d.expected, d.observed
        );
    }
    for s in &replay.stopped {
        let _ = writeln!(
            text,
            "fuzz spu-documented: {} stopped at instruction {}: {}",
            s.vector, s.step, s.reason
        );
    }
    let undefined: Vec<String> = replay
        .undefined
        .iter()
        .map(|(question, count)| format!("{}={count}", question.id()))
        .collect();
    let _ = writeln!(
        text,
        "fuzz spu-documented: {} vector(s), {} stopped, {} documented component(s), \
         {} differing; undefined {} [{}]",
        replay.vectors,
        replay.stopped.len(),
        replay.documented,
        replay.differences.len(),
        replay.undefined.values().sum::<usize>(),
        undefined.join(" "),
    );
    text
}
