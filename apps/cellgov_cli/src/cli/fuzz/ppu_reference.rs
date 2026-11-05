//! `dev fuzz ppu-reference`: replay every PPU reference file in a
//! directory and report the units with no file.

use std::fmt::Write as _;

use cellgov_fuzz::ppu_reference::{
    run_reference_directory, PpuReferenceCampaign, PpuReferenceCompleteness,
    PpuReferenceFileOutcome, PpuReferenceFileRun, PpuReferenceReplay,
};

use super::entry::write_stdout;
use super::error::FuzzCliError;
use crate::cli::exit::CommandExitCode;
use crate::cli::exit_codes;
use crate::cli::parse::FuzzPpuReferenceArgs;

/// Replays the directory, prints one line per vector and the
/// completeness verdict, and fails on any mismatch or gap.
pub(super) fn run_ppu_reference(
    args: &FuzzPpuReferenceArgs,
) -> Result<CommandExitCode, FuzzCliError> {
    let campaign = run_reference_directory(&args.dir)?;
    write_stdout(&render_ppu_reference(&campaign))?;
    Ok(if campaign.is_clean() {
        CommandExitCode::SUCCESS
    } else {
        CommandExitCode::new(exit_codes::FAILED)
    })
}

/// The campaign's lines: one per vector or refused file, one summary,
/// then one per gap.
pub(super) fn render_ppu_reference(campaign: &PpuReferenceCampaign) -> String {
    let mut text = String::new();
    let mut failing = 0usize;
    for run in &campaign.files {
        failing += render_file_run(&mut text, run);
    }
    render_summary(&mut text, campaign, failing);
    render_gaps(&mut text, &campaign.completeness);
    text
}

/// Writes one file's lines and returns its failing count. A refused file
/// counts as one failure.
fn render_file_run(text: &mut String, run: &PpuReferenceFileRun) -> usize {
    let unit = run.unit.as_deref().unwrap_or("none");
    match &run.outcome {
        PpuReferenceFileOutcome::Single(replay) => {
            let matched = render_vector(text, &run.file, unit, "single", replay);
            usize::from(!matched)
        }
        PpuReferenceFileOutcome::Set(replays) => {
            let mut failing = 0usize;
            for replay in replays {
                let matched = render_vector(text, &run.file, unit, &replay.name, &replay.replay);
                failing += usize::from(!matched);
            }
            failing
        }
        PpuReferenceFileOutcome::Refused(error) => {
            let _ = writeln!(
                text,
                "fuzz ppu-reference: {} unit={unit} refused: {error}",
                run.file
            );
            1
        }
    }
}

/// Writes one vector's line and returns whether it matched.
fn render_vector(
    text: &mut String,
    file: &str,
    unit: &str,
    name: &str,
    replay: &PpuReferenceReplay,
) -> bool {
    let matched = replay.is_match();
    let verdict = if matched { "match" } else { "differs" };
    let differences = replay
        .comparisons
        .iter()
        .flat_map(|comparison| &comparison.differences)
        .map(|difference| format!("{:?}", difference.field))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(",");
    let excluded = replay
        .comparisons
        .first()
        .map_or(0, |comparison| comparison.unrepresented.len());
    let paths = if replay.internal_divergence.is_some() {
        "diverge"
    } else {
        "agree"
    };
    let _ = writeln!(
        text,
        "fuzz ppu-reference: {file} {name} unit={unit} {verdict} differences=[{differences}] \
         paths={paths} excluded={excluded}"
    );
    matched
}

fn render_summary(text: &mut String, campaign: &PpuReferenceCampaign, failing: usize) {
    let completeness = &campaign.completeness;
    let _ = writeln!(
        text,
        "fuzz ppu-reference: {} file(s), {} vector(s), {failing} failing; units={} \
         covered={} pending={} missing={} duplicated={} unowned={} stale_pending={} \
         unknown_pending={}",
        campaign.files.len(),
        campaign.vectors(),
        completeness.units,
        completeness.covered,
        completeness.pending,
        completeness.missing.len(),
        completeness.duplicated.len(),
        completeness.unowned.len(),
        completeness.stale_pending.len(),
        completeness.unknown_pending.len(),
    );
}

fn render_gaps(text: &mut String, completeness: &PpuReferenceCompleteness) {
    for unit in &completeness.missing {
        let _ = writeln!(text, "fuzz ppu-reference: missing {unit}");
    }
    for (unit, files) in &completeness.duplicated {
        let _ = writeln!(
            text,
            "fuzz ppu-reference: duplicated {unit} in {}",
            files.join(", ")
        );
    }
    for file in &completeness.unowned {
        let _ = writeln!(text, "fuzz ppu-reference: unowned {file}");
    }
    for unit in &completeness.stale_pending {
        let _ = writeln!(text, "fuzz ppu-reference: stale pending {unit}");
    }
    for entry in &completeness.unknown_pending {
        let _ = writeln!(text, "fuzz ppu-reference: unknown pending {entry}");
    }
}
