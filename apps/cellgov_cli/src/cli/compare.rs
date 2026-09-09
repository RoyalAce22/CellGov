//! `diff`-family subcommand handlers: scenario/manifest compare,
//! observation diff, streaming state-trace divergence, and per-step
//! register-level zoom.

use cellgov_compare::{
    compare, compare_multi, format_human, format_json, format_multi_human, format_multi_json,
    observe_with_determinism_check, Classification, CompareMode, DeterminismError, Observation,
    RegionDescriptor,
};
use cellgov_testkit::fixtures::ScenarioFixture;

use super::exit::{die, load_file_or_die};
use super::parse::{die_usage, CompareArgs, OutputFormat};
use super::scenarios::scenario_factory;
use crate::cli::exit_codes;

// -- compare dispatch (top-level) --

pub(crate) fn run(args: &CompareArgs, format: OutputFormat, scenarios_list: &[&str]) {
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
        die_usage("--format applies to a run that produces a comparison report only");
    }

    if target.ends_with(".toml") {
        run_manifest_compare(
            target,
            mode,
            format,
            save_path,
            against_path,
            args.observations_dir.clone(),
        );
    } else {
        // Multi-observation compare needs a manifest's memory-region
        // descriptors. A bare scenario has none, so the flag would be
        // read and then never used.
        if args.observations_dir.is_some() {
            die_usage("--observations-dir applies to a manifest.toml target only");
        }
        match scenario_factory(target) {
            Some(factory) => {
                if let Some(path) = save_path.as_deref() {
                    // A bare scenario names no regions, so neither
                    // side of the round trip observes any.
                    save_baseline(&factory, target, path, &[]);
                } else if let Some(path) = against_path.as_deref() {
                    compare_against_baseline(&factory, target, path, mode, format);
                } else {
                    run_compare(&factory, target, mode, format);
                }
            }
            None => die(&format!(
                "unknown scenario: {target}\navailable: {}",
                scenarios_list.join(", ")
            )),
        }
    }
}

/// Observe `name` twice and die unless both runs yield the same observation.
///
/// An unreadable region fails both runs the same way. The message for
/// it names the region, so the operator can find its manifest line.
fn require_determinism(
    factory: &dyn Fn() -> ScenarioFixture,
    name: &str,
    regions: &[RegionDescriptor],
) -> Observation {
    observe_with_determinism_check(factory, regions).unwrap_or_else(|e| match e {
        DeterminismError::Observe(e) => die(&format!("observing {name}: {e}")),
        e => die(&format!("determinism check FAILED for {name}: {e}")),
    })
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
) {
    let obs = require_determinism(factory, name, regions);
    let p = std::path::Path::new(path);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).unwrap_or_else(|e| {
                die(&format!(
                    "failed to create baseline parent dir {}: {e}",
                    parent.display()
                ))
            });
        }
    }
    cellgov_compare::baseline::save(&obs, p)
        .unwrap_or_else(|e| die(&format!("failed to save baseline: {e:?}")));
    println!("saved baseline for {name} to {path}");
}

fn compare_against_baseline(
    factory: &dyn Fn() -> ScenarioFixture,
    name: &str,
    path: &str,
    mode: CompareMode,
    format: OutputFormat,
) {
    let obs = require_determinism(factory, name, &[]);
    let baseline = cellgov_compare::baseline::load(std::path::Path::new(path))
        .unwrap_or_else(|e| die(&format!("failed to load baseline from {path}: {e:?}")));

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
                format_json(&result, &baseline, &obs).expect(
                    "observation data is float-free by construction (style doc 0.5d contract)"
                )
            );
        }
    }
    if result.classification == Classification::Divergence {
        std::process::exit(exit_codes::FAILED);
    }
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
) {
    let obs = require_determinism(factory, name, &[]);
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
                serde_json::to_string_pretty(&obs).expect(
                    "observation data is float-free by construction (style doc 0.5d contract)"
                )
            );
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
) {
    let manifest = cellgov_compare::manifest::load(std::path::Path::new(manifest_path))
        .unwrap_or_else(|e| die(&format!("failed to load manifest {manifest_path}: {e:?}")));

    let test_name = &manifest.test.name;

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
            println!("test: {test_name}");
            println!("classification: UNSUPPORTED");
            println!("reason: no [cellgov] section in manifest");
            return;
        }
    };

    let factory = match scenario_factory(&cellgov_section.scenario) {
        Some(f) => f,
        None => {
            println!("test: {test_name}");
            println!("classification: UNSUPPORTED");
            println!(
                "reason: unknown CellGov scenario \"{}\"",
                cellgov_section.scenario
            );
            return;
        }
    };

    if let Some(path) = save_path {
        save_baseline(&factory, test_name, &path, &regions);
        return;
    }

    let obs = require_determinism(&factory, test_name, &regions);

    if let Some(dir) = observations_dir {
        let (baseline_paths, baselines): (Vec<std::path::PathBuf>, Vec<Observation>) =
            load_observations_with_paths(&dir).into_iter().unzip();
        if baselines.is_empty() {
            die(&format!("no observation .json files found in {dir}"));
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
                    format_multi_json(&result, &baselines, &obs).expect(
                        "observation data is float-free by construction (style doc 0.5d contract)"
                    )
                );
            }
        }
        if result.classification.exits_failure() {
            std::process::exit(exit_codes::FAILED);
        }
    } else if let Some(path) = against_path {
        let baseline = cellgov_compare::baseline::load(std::path::Path::new(&path))
            .unwrap_or_else(|e| die(&format!("failed to load baseline from {path}: {e:?}")));

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
                    format_json(&result, &baseline, &obs).expect(
                        "observation data is float-free by construction (style doc 0.5d contract)"
                    )
                );
            }
        }
        if result.classification == Classification::Divergence {
            std::process::exit(exit_codes::FAILED);
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
                    serde_json::to_string_pretty(&obs).expect(
                        "observation data is float-free by construction (style doc 0.5d contract)"
                    )
                );
            }
        }
    }
}

/// Load every `.json` observation in a directory, sorted by name.
/// Read failures die via [`die`].
pub(crate) fn load_observations_from_dir(dir: &str) -> Vec<Observation> {
    load_observations_with_paths(dir)
        .into_iter()
        .map(|(_, obs)| obs)
        .collect()
}

/// [`load_observations_from_dir`] with the file each observation was
/// read from, so a report can name the file.
fn load_observations_with_paths(dir: &str) -> Vec<(std::path::PathBuf, Observation)> {
    let rd = std::fs::read_dir(dir)
        .unwrap_or_else(|e| die(&format!("failed to read observations directory {dir}: {e}")));
    let mut entries: Vec<std::path::PathBuf> = Vec::new();
    for entry in rd {
        let entry = entry.unwrap_or_else(|e| {
            die(&format!(
                "observations directory {dir}: failed to read entry: {e}"
            ))
        });
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            entries.push(path);
        }
    }
    entries.sort();

    entries
        .into_iter()
        .map(|path| {
            let obs = cellgov_compare::baseline::load(&path).unwrap_or_else(|e| {
                die(&format!(
                    "failed to load observation {}: {e:?}",
                    path.display()
                ))
            });
            (path, obs)
        })
        .collect()
}

// -- diff observations --

/// Diff two JSON-encoded [`Observation`] files.
pub(crate) fn run_compare_observations(a_path: &str, b_path: &str, format: OutputFormat) {
    let a_bytes = load_file_or_die(a_path);
    let b_bytes = load_file_or_die(b_path);
    let a: cellgov_compare::Observation =
        serde_json::from_slice(&a_bytes).unwrap_or_else(|e| die(&format!("parse {a_path}: {e}")));
    let b: cellgov_compare::Observation =
        serde_json::from_slice(&b_bytes).unwrap_or_else(|e| die(&format!("parse {b_path}: {e}")));

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
                cellgov_compare::format_observation_compare_json(&result)
                    .expect("ObservationCompareResult is float-free by construction (style doc 0.5d contract)")
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
    if result.has_divergence() {
        std::process::exit(exit_codes::FAILED);
    }
}

// -- diverge --

/// Exit code: a state or zoom trace failed to decode, so no verdict
/// covers the records past the cut.
const EXIT_CORRUPT_TRACE: i32 = exit_codes::command_specific(31);

/// Exit code: `diff zoom` found the requested step in neither window.
const EXIT_MISSING_STEP: i32 = exit_codes::command_specific(30);

/// Streaming scan of two per-step state-trace files.
///
/// # Errors
///
/// Exit codes:
///
/// - 0 -- every `PpuStateHash` record matches.
/// - 1 -- a step verdict or a length verdict.
/// - [`EXIT_CORRUPT_TRACE`] -- a trace failed to decode before the
///   scan finished, so the scan prints no verdict for that file.
pub(crate) fn run_diverge(a_path: &str, b_path: &str) {
    use cellgov_compare::{diverge, DivergeField, DivergeReport, TraceDecodeError};
    let a_bytes = load_file_or_die(a_path);
    let b_bytes = load_file_or_die(b_path);
    report_trace_identity(&a_bytes, a_path, &b_bytes, b_path);
    match diverge(&a_bytes, &b_bytes) {
        DivergeReport::Identical { count } => {
            println!("IDENTICAL  {count} PpuStateHash records matched");
            if count == 0 {
                eprintln!(
                    "WARN: zero PpuStateHash records matched; trace files may be empty or truncated"
                );
            }
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
            std::process::exit(exit_codes::FAILED);
        }
        DivergeReport::LengthDiffers {
            common_count,
            a_count,
            b_count,
        } => {
            println!(
                "LENGTH_DIFFERS  common={common_count}  a={a_count}  b={b_count}  ({a_path} vs {b_path})"
            );
            std::process::exit(exit_codes::FAILED);
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
            std::process::exit(EXIT_CORRUPT_TRACE);
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
/// # Errors
///
/// Exit codes:
///
/// - 0 -- every fingerprint field and the PC agree at the step.
/// - 1 -- a register field or the PC differs.
/// - [`EXIT_MISSING_STEP`] -- one or both windows omit the step.
/// - [`EXIT_CORRUPT_TRACE`] -- a zoom trace failed to decode.
pub(crate) fn run_zoom(a_path: &str, b_path: &str, step: u64) {
    use cellgov_compare::{zoom_lookup, ZoomLookup};
    let a_bytes = load_file_or_die(a_path);
    let b_bytes = load_file_or_die(b_path);
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
                    std::process::exit(exit_codes::FAILED);
                }
                println!("NO_FIELD_DIFF step={step} pc=0x{a_pc:x}  snapshots agree on every fingerprint field and PC; if the hash stream diverged at this step, the harness is skewing snapshots against hashes -- investigate, do not resume the scan");
            } else {
                println!(
                    "ZOOM step={step} a_pc=0x{a_pc:x} b_pc=0x{b_pc:x}  {} field(s) differ:",
                    diffs.len()
                );
                for d in &diffs {
                    println!("  {:<5}  a=0x{:016x}  b=0x{:016x}", d.field, d.a, d.b);
                }
                std::process::exit(exit_codes::FAILED);
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
            std::process::exit(EXIT_MISSING_STEP);
        }
        ZoomLookup::CorruptTrace { a_error, b_error } => {
            let describe = |e: Option<String>| e.unwrap_or_else(|| "ok".into());
            println!(
                "CORRUPT_TRACE  a: {}  b: {}  (zoom file damaged; widening the window will not help)",
                describe(a_error),
                describe(b_error)
            );
            std::process::exit(EXIT_CORRUPT_TRACE);
        }
    }
}
