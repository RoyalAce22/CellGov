//! The `diff compare` entry: routes a manifest or a scenario target with its baseline flags.

use cellgov_compare::CompareMode;

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::exit_codes;
use crate::cli::parse::{CompareArgs, OutputFormat};
use crate::cli::scenarios::scenario_factory;

use super::manifest::run_manifest_compare;
use super::scenario::{compare_against_baseline, run_compare, save_baseline};

pub(crate) fn run(
    args: &CompareArgs,
    format: OutputFormat,
    scenarios_list: &[&str],
) -> Result<CommandExitCode, CommandError> {
    let target = args.target.as_str();
    let mode: CompareMode = args.mode.into();
    let save_path = args.save_baseline.clone();
    let against_path = args.against_baseline.clone();

    // `--save-baseline` declares `conflicts_with` on `--format`, which
    // catches only the trailing spelling. clap copies a global into a
    // subcommand's matches after it validates, so a `--format` ahead of
    // the subcommand never reaches the conflict check. A baseline run
    // prints no report, so the flag would be dropped.
    if save_path.is_some() && format != OutputFormat::Human {
        return Err(CommandError::status(
            exit_codes::USAGE,
            "--format applies to a run that produces a comparison report only",
        ));
    }

    if target.ends_with(".toml") {
        run_manifest_compare(
            target,
            mode,
            format,
            save_path,
            against_path,
            args.observations_dir.clone(),
            scenarios_list,
        )
    } else {
        // Multi-observation compare needs a manifest's memory-region
        // descriptors. A bare scenario has none, so the flag would be
        // read and then never used.
        if args.observations_dir.is_some() {
            return Err(CommandError::status(
                exit_codes::USAGE,
                "--observations-dir applies to a manifest.toml target only",
            ));
        }
        match scenario_factory(target) {
            Some(factory) => {
                if let Some(path) = save_path.as_deref() {
                    // A bare scenario names no regions, so neither
                    // side of the round trip observes any.
                    save_baseline(&factory, target, path, &[])?;
                    Ok(CommandExitCode::SUCCESS)
                } else if let Some(path) = against_path.as_deref() {
                    compare_against_baseline(&factory, target, path, mode, format)
                } else {
                    run_compare(&factory, target, mode, format)?;
                    Ok(CommandExitCode::SUCCESS)
                }
            }
            None => Err(CommandError::failed(format!(
                "unknown scenario: {target}\navailable: {}",
                scenarios_list.join(", ")
            ))),
        }
    }
}
