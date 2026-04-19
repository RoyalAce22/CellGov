//! All `compare`-family subcommand handlers:
//!
//! - `compare <scenario>` / `compare <manifest.toml>` -- run a
//!   scenario or manifest-driven compare against saved baselines.
//! - `compare-observations <a.json> <b.json>` -- diff two
//!   JSON-encoded `Observation` files.
//! - `diverge <a.state> <b.state>` -- streaming per-step state
//!   scan reporting the first divergence.
//! - `zoom <a.zoom.state> <b.zoom.state> <step>` -- per-field
//!   register diff at a named step.

use cellgov_compare::{
    compare, compare_multi, format_human, format_json, format_multi_human, format_multi_json,
    observe_with_determinism_check, Classification, CompareMode, Observation, RegionDescriptor,
};
use cellgov_testkit::fixtures::ScenarioFixture;

use super::args::{find_flag_value, parse_compare_mode, parse_output_format, OutputFormat};
use super::exit::{die, load_file_or_die};
use super::scenarios::scenario_factory;

// -- compare dispatch (top-level) --

/// Entry point for `cellgov_cli compare ...`.
pub(crate) fn run(args: &[String], scenarios_list: &[&str]) {
    let target = args.get(2).map(String::as_str).unwrap_or_else(|| {
        die(
            "usage: cellgov_cli compare <scenario|manifest.toml> [--mode strict|memory|events|prefix]",
        )
    });
    let mode = parse_compare_mode(args);
    let format = parse_output_format(args);
    let save_path = find_flag_value(args, "--save-baseline");
    let against_path = find_flag_value(args, "--against-baseline");
    let baselines_dir = find_flag_value(args, "--baselines-dir");

    if target.ends_with(".toml") {
        run_manifest_compare(target, mode, format, save_path, against_path, baselines_dir);
    } else {
        match scenario_factory(target) {
            Some(factory) => {
                if let Some(path) = save_path {
                    save_baseline(&factory, target, &path);
                } else if let Some(path) = against_path {
                    compare_against_baseline(&factory, target, &path, mode, format);
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

fn require_determinism(
    factory: &dyn Fn() -> ScenarioFixture,
    name: &str,
    regions: &[RegionDescriptor],
) -> Observation {
    observe_with_determinism_check(factory, regions)
        .unwrap_or_else(|e| die(&format!("determinism check FAILED for {name}: {e:?}")))
}

/// Run a scenario, observe it with determinism check, and save to disk.
fn save_baseline(factory: &dyn Fn() -> ScenarioFixture, name: &str, path: &str) {
    let obs = require_determinism(factory, name, &[]);
    let p = std::path::Path::new(path);
    // Propagate create_dir_all failures rather than dropping them
    // with `.ok()`. A permission error or a file-vs-directory
    // collision used to land as a lower-quality "No such file or
    // directory" from the subsequent save; naming the real cause
    // saves operators a round of diagnosis.
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

/// Run a scenario, observe it, load a saved baseline, and compare.
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
                format_json(&result, &baseline, &obs).expect("json serialization")
            );
        }
    }
    if result.classification == Classification::Divergence {
        std::process::exit(1);
    }
}

/// Run a scenario with determinism check and print the observation.
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
                serde_json::to_string_pretty(&obs).expect("json serialization")
            );
        }
    }
}

/// Manifest-driven comparison: load a manifest, run the CellGov
/// scenario if present, and compare against a saved baseline.
fn run_manifest_compare(
    manifest_path: &str,
    mode: CompareMode,
    format: OutputFormat,
    save_path: Option<String>,
    against_path: Option<String>,
    baselines_dir: Option<String>,
) {
    let manifest = cellgov_compare::manifest::load(std::path::Path::new(manifest_path))
        .unwrap_or_else(|e| die(&format!("failed to load manifest {manifest_path}: {e:?}")));

    let test_name = &manifest.test.name;

    // Build region descriptors from the manifest's [observe] section.
    let regions: Vec<RegionDescriptor> = manifest
        .observe
        .memory_regions
        .iter()
        .map(|r| RegionDescriptor {
            name: r.name.clone(),
            addr: r.addr,
            size: r.size,
        })
        .collect();

    // Run CellGov if the manifest has a [cellgov] section.
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
        save_baseline(&factory, test_name, &path);
        return;
    }

    let obs = require_determinism(&factory, test_name, &regions);

    if let Some(dir) = baselines_dir {
        let baselines = load_baselines_from_dir(&dir);
        if baselines.is_empty() {
            die(&format!("no baseline .json files found in {dir}"));
        }
        let result = compare_multi(&baselines, &obs, mode);
        match format {
            OutputFormat::Human => {
                println!("test: {test_name}");
                println!("manifest: {manifest_path}");
                println!("baselines-dir: {dir}");
                print!("{}", format_multi_human(&result, baselines.len()));
            }
            OutputFormat::Json => {
                println!(
                    "{}",
                    format_multi_json(&result, &baselines, &obs).expect("json serialization")
                );
            }
        }
        if matches!(
            result.classification,
            Classification::Divergence | Classification::UnsettledOracle
        ) {
            std::process::exit(1);
        }
    } else if let Some(path) = against_path {
        let baseline = cellgov_compare::baseline::load(std::path::Path::new(&path))
            .unwrap_or_else(|e| die(&format!("failed to load baseline from {path}: {e:?}")));

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
                    format_json(&result, &baseline, &obs).expect("json serialization")
                );
            }
        }
        if result.classification == Classification::Divergence {
            std::process::exit(1);
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
                    serde_json::to_string_pretty(&obs).expect("json serialization")
                );
            }
        }
    }
}

/// Load all `.json` baseline files from a directory, sorted by name.
/// Propagates `DirEntry` errors via [`die`] so a permission-denied
/// or broken-symlink entry does not silently disappear -- the
/// caller's "no baselines found" check would otherwise conflate
/// "none exist" with "all were unreadable".
pub(crate) fn load_baselines_from_dir(dir: &str) -> Vec<Observation> {
    let rd = std::fs::read_dir(dir)
        .unwrap_or_else(|e| die(&format!("failed to read baselines directory {dir}: {e}")));
    let mut entries: Vec<std::path::PathBuf> = Vec::new();
    for entry in rd {
        let entry = entry.unwrap_or_else(|e| {
            die(&format!(
                "baselines directory {dir}: failed to read entry: {e}"
            ))
        });
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            entries.push(path);
        }
    }
    entries.sort();

    entries
        .iter()
        .map(|path| {
            cellgov_compare::baseline::load(path).unwrap_or_else(|e| {
                die(&format!(
                    "failed to load baseline {}: {e:?}",
                    path.display()
                ))
            })
        })
        .collect()
}

// -- compare-observations --

/// `cellgov_cli compare-observations <a.json> <b.json>` -- diffs two
/// JSON-encoded `Observation` files (region-by-region byte equality
/// plus outcome match). Stops at the first divergence with a typed
/// label.
pub(crate) fn run_compare_observations(a_path: &str, b_path: &str) {
    let a_bytes = load_file_or_die(a_path);
    let b_bytes = load_file_or_die(b_path);
    let a: cellgov_compare::Observation =
        serde_json::from_slice(&a_bytes).unwrap_or_else(|e| die(&format!("parse {a_path}: {e}")));
    let b: cellgov_compare::Observation =
        serde_json::from_slice(&b_bytes).unwrap_or_else(|e| die(&format!("parse {b_path}: {e}")));

    if a.outcome != b.outcome {
        println!("DIVERGE outcome: {:?} vs {:?}", a.outcome, b.outcome);
        std::process::exit(1);
    }
    if a.memory_regions.len() != b.memory_regions.len() {
        println!(
            "DIVERGE region count: {} vs {}",
            a.memory_regions.len(),
            b.memory_regions.len()
        );
        std::process::exit(1);
    }
    for (ra, rb) in a.memory_regions.iter().zip(b.memory_regions.iter()) {
        if ra.name != rb.name || ra.addr != rb.addr {
            println!(
                "DIVERGE region identity: {}@0x{:x} vs {}@0x{:x}",
                ra.name, ra.addr, rb.name, rb.addr
            );
            std::process::exit(1);
        }
        // Length check FIRST: if one side is a prefix of the other,
        // `zip(...).position(x != y)` returns None and unwrap_or(0)
        // would report "first byte differs at 0x0" with identical
        // bytes.
        if ra.data.len() != rb.data.len() {
            println!(
                "DIVERGE region {}: length {} vs {} bytes",
                ra.name,
                ra.data.len(),
                rb.data.len()
            );
            std::process::exit(1);
        }
        if ra.data != rb.data {
            // Lengths are equal here, so `position` is guaranteed to
            // find the first differing byte when the slices differ.
            let first_diff = ra
                .data
                .iter()
                .zip(rb.data.iter())
                .position(|(x, y)| x != y)
                .expect("equal-length slices that differ must have a first diff");
            println!(
                "DIVERGE region {}: first byte differs at offset 0x{:x} (guest 0x{:x}) -- {:02x} vs {:02x}",
                ra.name,
                first_diff,
                ra.addr + first_diff as u64,
                ra.data[first_diff],
                rb.data[first_diff],
            );
            std::process::exit(1);
        }
    }
    let total_bytes: usize = a.memory_regions.iter().map(|r| r.data.len()).sum();
    println!(
        "MATCH outcome={:?}, {} regions ({} bytes) identical, steps {:?} vs {:?}",
        a.outcome,
        a.memory_regions.len(),
        total_bytes,
        a.metadata.steps,
        b.metadata.steps,
    );
    if a.memory_regions.is_empty() {
        eprintln!(
            "WARN: both observations carry zero memory regions; comparison is trivially vacuous"
        );
    }
    // Same runner + different step counts = determinism failure.
    // Cross-runner mismatch is expected (different step-count units).
    if let (Some(sa), Some(sb)) = (a.metadata.steps, b.metadata.steps) {
        if sa != sb {
            if a.metadata.runner == b.metadata.runner {
                println!(
                    "DIVERGE step count: {sa} vs {sb} within runner '{}' (byte-equal state reached via different work -- a determinism failure)",
                    a.metadata.runner
                );
                std::process::exit(1);
            }
            eprintln!(
                "NOTE: step counts differ ({sa} vs {sb}); cross-runner comparison between '{}' and '{}' does not require matching step counts",
                a.metadata.runner, b.metadata.runner
            );
        }
    }
}

// -- diverge --

/// `cellgov_cli diverge <a.state> <b.state>` -- streaming scan of two
/// per-step state-trace files. Prints IDENTICAL / DIVERGE / LENGTH_DIFFERS
/// and exits non-zero on any non-identical outcome.
pub(crate) fn run_diverge(a_path: &str, b_path: &str) {
    use cellgov_compare::{diverge, DivergeField, DivergeReport};
    let a_bytes = load_file_or_die(a_path);
    let b_bytes = load_file_or_die(b_path);
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
            std::process::exit(1);
        }
        DivergeReport::LengthDiffers {
            common_count,
            a_count,
            b_count,
        } => {
            println!(
                "LENGTH_DIFFERS  common={common_count}  a={a_count}  b={b_count}  ({a_path} vs {b_path})"
            );
            std::process::exit(1);
        }
    }
}

// -- zoom --

/// `cellgov_cli zoom <a.zoom.state> <b.zoom.state> <step>` -- loads
/// two zoom-trace byte streams, runs `cellgov_compare::zoom_lookup`
/// at the named step, and prints a per-field register diff.
///
/// Exit codes: 0 on identical-state hash collision, 1 on a real diff,
/// 2 when the requested step is missing from one or both windows.
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
                println!("HASH_COLLISION step={step} pc=0x{a_pc:x}  full snapshots are byte-equal; resume scan from step {next}", next = step + 1);
            } else {
                println!(
                    "ZOOM step={step} a_pc=0x{a_pc:x} b_pc=0x{b_pc:x}  {} field(s) differ:",
                    diffs.len()
                );
                for d in &diffs {
                    println!("  {:<5}  a=0x{:016x}  b=0x{:016x}", d.field, d.a, d.b);
                }
                std::process::exit(1);
            }
        }
        ZoomLookup::MissingStep {
            step,
            a_missing,
            b_missing,
        } => {
            // Positive-sense locals so the format string reads
            // cleanly; the `!` lives in exactly one place.
            let a_has_step = !a_missing;
            let b_has_step = !b_missing;
            println!(
                "MISSING_STEP step={step}  a_has_step={a_has_step}  b_has_step={b_has_step}  (zoom window did not cover this step on at least one side)"
            );
            std::process::exit(2);
        }
    }
}
