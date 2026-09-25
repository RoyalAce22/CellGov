//! `diff observations`, and loading a directory of observations.

use cellgov_compare::Observation;

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::exit_codes;
use crate::cli::parse::OutputFormat;
use crate::cli::self_load::load_file;

/// Load every `.json` observation in a directory, sorted by name.
///
/// # Errors
///
/// Returns an error if the command cannot load an observation.
pub(crate) fn load_observations_from_dir(dir: &str) -> Result<Vec<Observation>, CommandError> {
    Ok(load_observations_with_paths(dir)?
        .into_iter()
        .map(|(_, obs)| obs)
        .collect())
}

/// [`load_observations_from_dir`] with the file each observation was
/// read from, so a report can name the file.
pub(super) fn load_observations_with_paths(
    dir: &str,
) -> Result<Vec<(std::path::PathBuf, Observation)>, CommandError> {
    cellgov_compare::baseline::load_dir(std::path::Path::new(dir))
        .map_err(|error| CommandError::failed(error.to_string()))
}

/// Diff two JSON-encoded [`Observation`] files.
///
/// # Errors
///
/// Returns an error if the command cannot load an input.
pub(crate) fn run_compare_observations(
    a_path: &str,
    b_path: &str,
    format: OutputFormat,
) -> Result<CommandExitCode, CommandError> {
    let a_bytes = load_file(a_path)?;
    let b_bytes = load_file(b_path)?;
    let a = serde_json::from_slice(&a_bytes)
        .map_err(|error| CommandError::failed(format!("parse {a_path}: {error}")))?;
    let b = serde_json::from_slice(&b_bytes)
        .map_err(|error| CommandError::failed(format!("parse {b_path}: {error}")))?;

    let result = cellgov_compare::compare_observations(&a, &b);
    // The report prints before the verdict. A reader who stops at the
    // first line still learns whether the store composed the two sides
    // the same way.
    for line in result.identity_report(a_path, b_path) {
        eprintln!("{line}");
    }
    match format {
        OutputFormat::Human => {
            print!(
                "{}",
                cellgov_compare::format_observation_compare_human(&result)
            );
            if result.is_vacuous() {
                eprintln!(
                    "WARN: both observations carry zero memory regions; nothing was compared"
                );
            }
            if let Some((sa, sb)) = result.cross_runner_step_note() {
                eprintln!(
                    "NOTE: step counts differ ({sa} vs {sb}); cross-runner comparison between '{}' and '{}' does not require matching step counts",
                    result.a_runner, result.b_runner,
                );
            }
        }
        OutputFormat::Json => {
            // WARN / NOTE stay stderr-only; stdout must remain a
            // machine-parseable JSON payload.
            println!(
                "{}",
                cellgov_compare::format_observation_compare_json(&result).map_err(|error| {
                    CommandError::failed(format!("compare: encode cross-runner JSON: {error}"))
                })?
            );
            if result.is_vacuous() {
                eprintln!(
                    "WARN: both observations carry zero memory regions; nothing was compared"
                );
            }
            if let Some((sa, sb)) = result.cross_runner_step_note() {
                eprintln!(
                    "NOTE: step counts differ ({sa} vs {sb}); cross-runner comparison between '{}' and '{}' does not require matching step counts",
                    result.a_runner, result.b_runner,
                );
            }
        }
    }
    Ok(CommandExitCode::new(if result.has_divergence() {
        exit_codes::FAILED
    } else if result.scheme_mismatch().is_some() {
        super::scenario::EXIT_SCHEME_MISMATCH
    } else {
        0
    }))
}
