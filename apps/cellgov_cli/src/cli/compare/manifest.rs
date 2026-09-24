//! Manifest compare, against one baseline or every observation in a directory.

use cellgov_compare::{
    compare, compare_multi, format_human, format_json, format_multi_human, format_multi_json,
    Classification, CompareMode, Observation, RegionDescriptor,
};

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::exit_codes;
use crate::cli::parse::OutputFormat;
use crate::cli::scenarios::scenario_factory;

use super::observations::load_observations_with_paths;
use super::scenario::{report_identity, require_determinism, save_baseline};

/// Report, or refuse, a manifest that names no scenario this runner has.
///
/// A plain run reports it: UNSUPPORTED is the classification for a
/// manifest another runner owns. A run with a baseline flag has
/// nothing to record and nothing to compare, so it refuses instead of
/// exiting green.
fn unsupported_manifest(
    manifest_path: &str,
    test_name: &str,
    reason: &str,
    baseline_flag: Option<&str>,
) -> Result<(), CommandError> {
    match baseline_flag {
        Some(flag) => Err(CommandError::failed(format!(
            "manifest {manifest_path}: {reason}; {flag} needs a CellGov run, and this manifest names none"
        ))),
        None => {
            println!("test: {test_name}");
            println!("classification: UNSUPPORTED");
            println!("reason: {reason}");
            Ok(())
        }
    }
}

pub(super) fn run_manifest_compare(
    manifest_path: &str,
    mode: CompareMode,
    format: OutputFormat,
    save_path: Option<String>,
    against_path: Option<String>,
    observations_dir: Option<String>,
    scenarios_list: &[&str],
) -> Result<CommandExitCode, CommandError> {
    let manifest =
        cellgov_compare::manifest::load(std::path::Path::new(manifest_path)).map_err(|error| {
            CommandError::failed(format!(
                "failed to load manifest {manifest_path}: {error:?}"
            ))
        })?;

    let test_name = &manifest.test.name;

    // Whichever flag asked for a baseline names itself in a refusal.
    let baseline_flag = if save_path.is_some() {
        Some("--save-baseline")
    } else if against_path.is_some() {
        Some("--against-baseline")
    } else if observations_dir.is_some() {
        Some("--observations-dir")
    } else {
        None
    };

    let regions: Vec<RegionDescriptor> = manifest.observe.region_descriptors();

    let cellgov_section = match &manifest.cellgov {
        Some(cg) => cg,
        None => {
            unsupported_manifest(
                manifest_path,
                test_name,
                "no [cellgov] section in manifest",
                baseline_flag,
            )?;
            return Ok(CommandExitCode::SUCCESS);
        }
    };

    let factory = match scenario_factory(&cellgov_section.scenario) {
        Some(f) => f,
        None => {
            unsupported_manifest(
                manifest_path,
                test_name,
                &format!(
                    "unknown CellGov scenario \"{}\" (available: {})",
                    cellgov_section.scenario,
                    scenarios_list.join(", ")
                ),
                baseline_flag,
            )?;
            return Ok(CommandExitCode::SUCCESS);
        }
    };

    if let Some(path) = save_path {
        save_baseline(&factory, test_name, &path, &regions)?;
        return Ok(CommandExitCode::SUCCESS);
    }

    let obs = require_determinism(&factory, test_name, &regions)?;

    if let Some(dir) = observations_dir {
        let (baseline_paths, baselines): (Vec<std::path::PathBuf>, Vec<Observation>) =
            load_observations_with_paths(&dir)?.into_iter().unzip();
        if baselines.is_empty() {
            return Err(CommandError::failed(format!(
                "no observation .json files found in {dir}"
            )));
        }
        for (path, baseline) in baseline_paths.iter().zip(&baselines) {
            report_identity(baseline, &path.display().to_string(), &obs, test_name);
        }
        let result = compare_multi(&baselines, &obs, mode);
        match format {
            OutputFormat::Human => {
                println!("test: {test_name}");
                println!("manifest: {manifest_path}");
                println!("observations-dir: {dir}");
                print!("{}", format_multi_human(&result, baselines.len()));
            }
            OutputFormat::Json => {
                println!(
                    "{}",
                    format_multi_json(&result, &baselines, &obs).map_err(|error| {
                        CommandError::failed(format!(
                            "compare: encode multi-baseline JSON: {error}"
                        ))
                    })?
                );
            }
        }
        if result.classification.exits_failure() {
            return Ok(CommandExitCode::new(exit_codes::FAILED));
        }
    } else if let Some(path) = against_path {
        let baseline =
            cellgov_compare::baseline::load(std::path::Path::new(&path)).map_err(|error| {
                CommandError::failed(format!("failed to load baseline from {path}: {error:?}"))
            })?;

        report_identity(&baseline, &path, &obs, test_name);
        let result = compare(&baseline, &obs, mode);
        match format {
            OutputFormat::Human => {
                println!("test: {test_name}");
                println!("manifest: {manifest_path}");
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
        if result.classification == Classification::Divergence {
            return Ok(CommandExitCode::new(exit_codes::FAILED));
        }
    } else {
        match format {
            OutputFormat::Human => {
                println!("test: {test_name}");
                println!("manifest: {manifest_path}");
                println!("determinism: ok");
                println!("outcome: {:?}", obs.outcome);
                println!("events: {}", obs.events.len());
                println!("regions: {}", obs.memory_regions.len());
                for region in &obs.memory_regions {
                    println!(
                        "  {} addr=0x{:x} size={}",
                        region.name,
                        region.addr,
                        region.data.len()
                    );
                }
                if let Some(hashes) = &obs.state_hashes {
                    println!("memory_hash: 0x{:016x}", hashes.memory.raw());
                    println!("status_hash: 0x{:016x}", hashes.unit_status.raw());
                    println!("sync_hash: 0x{:016x}", hashes.sync.raw());
                }
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
    }
    Ok(CommandExitCode::SUCCESS)
}
