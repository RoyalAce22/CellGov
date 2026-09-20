//! `diff`-family subcommand handlers: scenario/manifest compare,
//! observation diff, streaming state-trace divergence, and per-step
//! register-level zoom.

use cellgov_compare::{
    compare, compare_multi, format_human, format_json, format_multi_human, format_multi_json,
    observe_checked, Classification, CompareMode, DeterminismError, Observation, RegionDescriptor,
};
use cellgov_testkit::fixtures::ScenarioFixture;

use super::exit::{CommandError, CommandExitCode};
use super::parse::{CompareArgs, OutputFormat};
use super::scenarios::scenario_factory;
use super::self_load::load_file;
use crate::cli::exit_codes;

// -- compare dispatch (top-level) --

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

/// Checks that two observations of `name` match.
///
/// # Errors
///
/// - Returns status 1 if both runs refuse the same region.
/// - Returns status 3 if the observations differ.
fn require_determinism(
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
pub(super) fn report_first_invariant_break(line: Option<&str>) {
    if let Some(line) = line {
        eprintln!("warning: {line}");
    }
}

/// Distinguishes an observation refusal from disagreement between two runs.
fn determinism_exit_status(e: &DeterminismError) -> i32 {
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
fn save_baseline(
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

fn compare_against_baseline(
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

fn report_identity(a: &Observation, a_label: &str, b: &Observation, b_label: &str) {
    for line in cellgov_compare::identity_report(&a.identity, a_label, &b.identity, b_label) {
        eprintln!("{line}");
    }
}

fn run_compare(
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

fn run_manifest_compare(
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

    let regions: Vec<RegionDescriptor> = manifest
        .observe
        .memory_regions
        .iter()
        .map(|r| RegionDescriptor {
            name: r.name.clone(),
            space: cellgov_compare::AddressSpaceId::new(r.space),
            addr: r.addr,
            size: r.size,
        })
        .collect();

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
fn load_observations_with_paths(
    dir: &str,
) -> Result<Vec<(std::path::PathBuf, Observation)>, CommandError> {
    let rd = std::fs::read_dir(dir).map_err(|error| {
        CommandError::failed(format!(
            "failed to read observations directory {dir}: {error}"
        ))
    })?;
    let mut entries: Vec<std::path::PathBuf> = Vec::new();
    for entry in rd {
        let entry = entry.map_err(|error| {
            CommandError::failed(format!(
                "observations directory {dir}: failed to read entry: {error}"
            ))
        })?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            entries.push(path);
        }
    }
    entries.sort();

    entries
        .into_iter()
        .map(|path| {
            cellgov_compare::baseline::load(&path)
                .map(|obs| (path.clone(), obs))
                .map_err(|error| {
                    CommandError::failed(format!(
                        "failed to load observation {}: {error:?}",
                        path.display()
                    ))
                })
        })
        .collect()
}

// -- diff observations --

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
    } else {
        0
    }))
}

// -- diverge --

/// Exit code: a state or zoom trace failed to decode, so no verdict
/// covers the records past the cut.
const EXIT_CORRUPT_TRACE: i32 = exit_codes::command_specific(31);

/// Exit code: `diff zoom` found the requested step in neither window.
const EXIT_MISSING_STEP: i32 = exit_codes::command_specific(30);

/// Streaming scan of two per-step state-trace files.
///
/// # Exit status
///
/// - Status 0 means every `PpuStateHash` record matches.
/// - Status 1 means the step or trace length differs.
/// - [`EXIT_CORRUPT_TRACE`] means a trace failed to decode before the
///   scan finished. The scan prints no verdict for that file.
///
/// # Errors
///
/// Returns an error if a trace file cannot be read.
pub(crate) fn run_diverge(a_path: &str, b_path: &str) -> Result<CommandExitCode, CommandError> {
    use cellgov_compare::{diverge, DivergeField, DivergeReport, TraceDecodeError};
    let a_bytes = load_file(a_path)?;
    let b_bytes = load_file(b_path)?;
    report_trace_identity(&a_bytes, a_path, &b_bytes, b_path);
    match diverge(&a_bytes, &b_bytes) {
        DivergeReport::Identical { count } => {
            println!("IDENTICAL  {count} PpuStateHash records matched");
            if count == 0 {
                eprintln!(
                    "WARN: zero PpuStateHash records matched; trace files may be empty or truncated"
                );
            }
            Ok(CommandExitCode::SUCCESS)
        }
        DivergeReport::Differs {
            step,
            a_pc,
            b_pc,
            a_hash,
            b_hash,
            field,
        } => {
            let field_str = match field {
                DivergeField::Pc => "pc",
                DivergeField::Hash => "hash",
            };
            println!(
                "DIVERGE step={step} field={field_str}  a_pc=0x{a_pc:x} b_pc=0x{b_pc:x}  a_hash=0x{a_hash:x} b_hash=0x{b_hash:x}"
            );
            Ok(CommandExitCode::new(exit_codes::FAILED))
        }
        DivergeReport::LengthDiffers {
            common_count,
            a_count,
            b_count,
        } => {
            println!(
                "LENGTH_DIFFERS  common={common_count}  a={a_count}  b={b_count}  ({a_path} vs {b_path})"
            );
            Ok(CommandExitCode::new(exit_codes::FAILED))
        }
        DivergeReport::CorruptTrace {
            common_count,
            a_error,
            b_error,
        } => {
            let describe =
                |e: Option<TraceDecodeError>| e.map_or_else(|| "ok".into(), |e| e.to_string());
            println!(
                "CORRUPT_TRACE  common={common_count}  a: {}  b: {}  (a state file failed to decode; the {common_count} records before the cut matched and nothing past it was compared)",
                describe(a_error),
                describe(b_error)
            );
            Ok(CommandExitCode::new(EXIT_CORRUPT_TRACE))
        }
    }
}

fn report_trace_identity(a: &[u8], a_path: &str, b: &[u8], b_path: &str) {
    for line in cellgov_compare::cross_trace_identity_warning(
        cellgov_compare::trace_identity(a),
        a_path,
        cellgov_compare::trace_identity(b),
        b_path,
    ) {
        eprintln!("{line}");
    }
}

// -- zoom --

/// Per-field register diff at the named step.
///
/// # Exit status
///
/// - Status 0 means every fingerprint field and the PC agree.
/// - Status 1 means a register field or the PC differs.
/// - [`EXIT_MISSING_STEP`] means one or both windows omit the step.
/// - [`EXIT_CORRUPT_TRACE`] means a zoom trace failed to decode.
///
/// # Errors
///
/// Returns an error if a trace file cannot be read.
pub(crate) fn run_zoom(
    a_path: &str,
    b_path: &str,
    step: u64,
) -> Result<CommandExitCode, CommandError> {
    use cellgov_compare::{zoom_lookup, ZoomLookup};
    let a_bytes = load_file(a_path)?;
    let b_bytes = load_file(b_path)?;
    match zoom_lookup(&a_bytes, &b_bytes, step) {
        ZoomLookup::Found {
            step,
            a_pc,
            b_pc,
            diffs,
        } => {
            if diffs.is_empty() {
                // PC is outside the fingerprint, so `diff diverge` can name
                // a Pc divergence whose zoom diff is empty -- that is
                // a real control-flow divergence, not harness skew.
                if a_pc != b_pc {
                    println!(
                        "PC_DIFF step={step} a_pc=0x{a_pc:x} b_pc=0x{b_pc:x}  registers agree but control flow diverged; the PC split is the divergence"
                    );
                    return Ok(CommandExitCode::new(exit_codes::FAILED));
                }
                println!("NO_FIELD_DIFF step={step} pc=0x{a_pc:x}  snapshots agree on every fingerprint field and PC; if the hash stream diverged at this step, the harness is skewing snapshots against hashes -- investigate, do not resume the scan");
                Ok(CommandExitCode::SUCCESS)
            } else {
                println!(
                    "ZOOM step={step} a_pc=0x{a_pc:x} b_pc=0x{b_pc:x}  {} field(s) differ:",
                    diffs.len()
                );
                for d in &diffs {
                    println!("  {:<5}  a=0x{:016x}  b=0x{:016x}", d.field, d.a, d.b);
                }
                Ok(CommandExitCode::new(exit_codes::FAILED))
            }
        }
        ZoomLookup::MissingStep {
            step,
            a_missing,
            b_missing,
        } => {
            let a_has_step = !a_missing;
            let b_has_step = !b_missing;
            println!(
                "MISSING_STEP step={step}  a_has_step={a_has_step}  b_has_step={b_has_step}  (zoom window did not cover this step on at least one side)"
            );
            Ok(CommandExitCode::new(EXIT_MISSING_STEP))
        }
        ZoomLookup::CorruptTrace { a_error, b_error } => {
            let describe = |e: Option<String>| e.unwrap_or_else(|| "ok".into());
            println!(
                "CORRUPT_TRACE  a: {}  b: {}  (zoom file damaged; widening the window will not help)",
                describe(a_error),
                describe(b_error)
            );
            Ok(CommandExitCode::new(EXIT_CORRUPT_TRACE))
        }
    }
}

#[cfg(test)]
#[path = "tests/compare_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/compare_disagreement_tests.rs"]
mod disagreement_tests;
