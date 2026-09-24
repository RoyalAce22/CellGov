//! Scenario compare: the determinism check, baseline save and load, and the report.

use cellgov_compare::{
    compare, format_human, format_json, observe_checked, Classification, CompareMode,
    DeterminismError, Observation, RegionDescriptor,
};
use cellgov_testkit::fixtures::ScenarioFixture;

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::exit_codes;
use crate::cli::parse::OutputFormat;

/// Checks that two observations of `name` match.
///
/// # Errors
///
/// - Returns status 1 if both runs refuse the same region.
/// - Returns status 3 if the observations differ.
pub(super) fn require_determinism(
    factory: &dyn Fn() -> ScenarioFixture,
    name: &str,
    regions: &[RegionDescriptor],
) -> Result<Observation, CommandError> {
    let run = observe_checked(factory, regions).map_err(|error| {
        let message = match &error {
            DeterminismError::Observe(error) => format!("observing {name}: {error}"),
            error => format!("determinism break for {name}: {error}"),
        };
        CommandError::status(determinism_exit_status(&error), message)
    })?;
    report_first_invariant_break(run.first_invariant_break.as_deref());
    Ok(run.observation)
}

/// Report a run's first LV2 host invariant break, the line every
/// driver reports. `explore` reports through this same helper.
///
/// The line goes to stderr so it survives a `--format json` run, whose
/// stdout a reader parses.
pub(in crate::cli) fn report_first_invariant_break(line: Option<&str>) {
    if let Some(line) = line {
        eprintln!("warning: {line}");
    }
}

/// Distinguishes an observation refusal from disagreement between two runs.
pub(super) fn determinism_exit_status(e: &DeterminismError) -> i32 {
    match e {
        DeterminismError::Observe(_) => exit_codes::FAILED,
        DeterminismError::ObserveDisagreement(_)
        | DeterminismError::OutcomeMismatch
        | DeterminismError::MemoryMismatch
        | DeterminismError::EventMismatch
        | DeterminismError::HashMismatch => exit_codes::DISAGREED,
    }
}

/// `regions` must match what the `--against-baseline` run observes with.
///
/// A comparison matches regions by name. A baseline saved without the
/// caller's regions reads every region the compare run captures as a
/// divergence.
pub(super) fn save_baseline(
    factory: &dyn Fn() -> ScenarioFixture,
    name: &str,
    path: &str,
    regions: &[RegionDescriptor],
) -> Result<(), CommandError> {
    let obs = require_determinism(factory, name, regions)?;
    let p = std::path::Path::new(path);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| {
                CommandError::failed(format!(
                    "failed to create baseline parent dir {}: {error}",
                    parent.display()
                ))
            })?;
        }
    }
    cellgov_compare::baseline::save(&obs, p)
        .map_err(|error| CommandError::failed(format!("failed to save baseline: {error:?}")))?;
    println!("saved baseline for {name} to {path}");
    Ok(())
}

pub(super) fn compare_against_baseline(
    factory: &dyn Fn() -> ScenarioFixture,
    name: &str,
    path: &str,
    mode: CompareMode,
    format: OutputFormat,
) -> Result<CommandExitCode, CommandError> {
    let obs = require_determinism(factory, name, &[])?;
    let baseline =
        cellgov_compare::baseline::load(std::path::Path::new(path)).map_err(|error| {
            CommandError::failed(format!("failed to load baseline from {path}: {error:?}"))
        })?;

    report_identity(&baseline, path, &obs, name);
    let result = compare(&baseline, &obs, mode);
    match format {
        OutputFormat::Human => {
            println!("scenario: {name}");
            println!("baseline: {path}");
            print!("{}", format_human(&result));
        }
        OutputFormat::Json => {
            println!(
                "{}",
                format_json(&result, &baseline, &obs).map_err(|error| {
                    CommandError::failed(format!("compare: encode JSON: {error}"))
                })?
            );
        }
    }
    Ok(CommandExitCode::new(
        if result.classification == Classification::Divergence {
            exit_codes::FAILED
        } else {
            0
        },
    ))
}

pub(super) fn report_identity(a: &Observation, a_label: &str, b: &Observation, b_label: &str) {
    for line in cellgov_compare::identity_report(&a.identity, a_label, &b.identity, b_label) {
        eprintln!("{line}");
    }
}

pub(super) fn run_compare(
    factory: &dyn Fn() -> ScenarioFixture,
    name: &str,
    mode: CompareMode,
    format: OutputFormat,
) -> Result<(), CommandError> {
    let obs = require_determinism(factory, name, &[])?;
    match format {
        OutputFormat::Human => {
            println!("scenario: {name}");
            println!("determinism: ok");
            println!("outcome: {:?}", obs.outcome);
            println!("events: {}", obs.events.len());
            for event in &obs.events {
                println!(
                    "  {:4}  {:?} unit={}",
                    event.sequence, event.kind, event.unit
                );
            }
            if let Some(hashes) = &obs.state_hashes {
                println!("memory_hash: 0x{:016x}", hashes.memory.raw());
                println!("status_hash: 0x{:016x}", hashes.unit_status.raw());
                println!("sync_hash: 0x{:016x}", hashes.sync.raw());
            }
            println!("mode: {mode:?}");
            println!("steps: {}", obs.metadata.steps.unwrap_or(0));
        }
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&obs).map_err(|error| {
                    CommandError::failed(format!("compare: encode observation JSON: {error}"))
                })?
            );
        }
    }
    Ok(())
}
