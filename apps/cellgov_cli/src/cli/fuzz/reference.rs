//! `dev fuzz spu-reference`: replay every SPU reference file in a
//! directory and report the units with no file.

use std::fmt::Write as _;

use cellgov_fuzz::spu_reference::{
    run_reference_directory, SpuReferenceCampaign, SpuReferenceFileOutcome,
};

use super::entry::write_stdout;
use super::error::FuzzCliError;
use crate::cli::exit::CommandExitCode;
use crate::cli::exit_codes;
use crate::cli::parse::FuzzSpuReferenceArgs;

/// Replays the directory, prints one line per vector and the
/// completeness verdict, and fails on any mismatch or gap.
pub(super) fn run_spu_reference(
    args: &FuzzSpuReferenceArgs,
) -> Result<CommandExitCode, FuzzCliError> {
    let campaign = run_reference_directory(&args.dir)?;
    write_stdout(&render_spu_reference(&campaign))?;
    Ok(if campaign.is_clean() {
        CommandExitCode::SUCCESS
    } else {
        CommandExitCode::new(exit_codes::FAILED)
    })
}

/// The campaign's lines: one per vector or refused file, one summary,
/// then one per gap.
pub(super) fn render_spu_reference(campaign: &SpuReferenceCampaign) -> String {
    let mut text = String::new();
    let mut vectors = 0usize;
    let mut mismatched = 0usize;
    for run in &campaign.files {
        let unit = run.unit.as_deref().unwrap_or("none");
        let mut vector =
            |name: &str, matched: bool, differences: String, unchosen: String, excluded: usize| {
                vectors += 1;
                let verdict = if matched {
                    "match"
                } else {
                    mismatched += 1;
                    "differs"
                };
                let _ = writeln!(
                text,
                "fuzz spu-reference: {} {name} unit={unit} {verdict} differences=[{differences}] \
                 unchosen=[{unchosen}] excluded={excluded}",
                run.file
            );
            };
        match &run.outcome {
            SpuReferenceFileOutcome::Single(replay) => {
                let comparison = &replay.comparison;
                vector(
                    "single",
                    comparison.is_match(),
                    join(comparison.differences.iter()),
                    String::new(),
                    comparison.unrepresented.len(),
                );
            }
            SpuReferenceFileOutcome::Set(replays) => {
                for replay in replays {
                    let comparison = &replay.comparison;
                    vector(
                        &replay.name,
                        comparison.is_match(),
                        join(comparison.differences.iter()),
                        join(comparison.unchosen.iter()),
                        comparison.unrepresented.len(),
                    );
                }
            }
            SpuReferenceFileOutcome::Refused(error) => {
                mismatched += 1;
                let _ = writeln!(
                    text,
                    "fuzz spu-reference: {} unit={unit} refused: {error}",
                    run.file
                );
            }
        }
    }
    let completeness = &campaign.completeness;
    let _ = writeln!(
        text,
        "fuzz spu-reference: {} file(s), {vectors} vector(s), {mismatched} failing; units={} \
         covered={} pending={} missing={} duplicated={} unowned={} stale_pending={} \
         unknown_pending={}",
        campaign.files.len(),
        completeness.units,
        completeness.covered,
        completeness.pending,
        completeness.missing.len(),
        completeness.duplicated.len(),
        completeness.unowned.len(),
        completeness.stale_pending.len(),
        completeness.unknown_pending.len(),
    );
    for unit in &completeness.missing {
        let _ = writeln!(text, "fuzz spu-reference: missing {unit}");
    }
    for (unit, files) in &completeness.duplicated {
        let _ = writeln!(
            text,
            "fuzz spu-reference: duplicated {unit} in {}",
            files.join(", ")
        );
    }
    for file in &completeness.unowned {
        let _ = writeln!(text, "fuzz spu-reference: unowned {file}");
    }
    for unit in &completeness.stale_pending {
        let _ = writeln!(text, "fuzz spu-reference: stale pending {unit}");
    }
    for entry in &completeness.unknown_pending {
        let _ = writeln!(text, "fuzz spu-reference: unknown pending {entry}");
    }
    text
}

fn join<T: std::fmt::Debug>(items: impl Iterator<Item = T>) -> String {
    items
        .map(|item| format!("{item:?}"))
        .collect::<Vec<_>>()
        .join(",")
}
