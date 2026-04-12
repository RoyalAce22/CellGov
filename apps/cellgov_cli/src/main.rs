//! cellgov_cli -- run scenarios, dump traces, compare observations.
//!
//! Commands:
//!
//! - `cellgov_cli <scenario>` -- run a scenario, print deterministic summary.
//! - `cellgov_cli dump <scenario>` -- run a scenario, print every trace record.
//! - `cellgov_cli compare <scenario> [--mode strict|memory|events|prefix]` --
//!   run a scenario with determinism check and print the observation.
//! - `cellgov_cli compare <manifest.toml> [--mode ...] [--against-baseline <path>]` --
//!   manifest-driven comparison: run CellGov scenario from manifest and
//!   compare against a saved baseline.
//!
//! Available scenarios: fairness, conflict, mailbox, dma, send, signal, isa.

use cellgov_compare::{
    compare, compare_multi, format_human, format_json, format_multi_human, format_multi_json,
    observe_with_determinism_check, Classification, CompareMode, Observation, RegionDescriptor,
};
use cellgov_testkit::fixtures::{self, ScenarioFixture};
use cellgov_testkit::runner::{run, ScenarioOutcome, ScenarioResult};
use cellgov_trace::TraceReader;

/// Supported scenario names and the fixture factories that produce them.
fn run_scenario(name: &str) -> Option<(&str, ScenarioResult)> {
    let (label, fixture) = match name {
        "fairness" => (
            "round-robin-fairness(3 units, 5 steps each)",
            fixtures::round_robin_fairness_scenario(3, 5),
        ),
        "conflict" => (
            "write-conflict(3 steps each)",
            fixtures::write_conflict_scenario(3),
        ),
        "mailbox" => (
            "mailbox-roundtrip(command=0x42)",
            fixtures::mailbox_roundtrip_scenario(0x42),
        ),
        "dma" => ("dma-block-unblock", fixtures::dma_block_unblock_scenario()),
        "send" => (
            "mailbox-send(5 messages)",
            fixtures::mailbox_send_scenario(5),
        ),
        "signal" => ("signal-update(4 bits)", fixtures::signal_update_scenario(4)),
        "isa" => ("fake-isa-integration", fixtures::fake_isa_scenario()),
        _ => return None,
    };
    Some((label, run(fixture)))
}

/// Return a closure that builds a fresh ScenarioFixture for the named
/// scenario. Used by the compare command which needs to run the
/// scenario twice for the determinism check.
fn scenario_factory(name: &str) -> Option<Box<dyn Fn() -> ScenarioFixture>> {
    let factory: Box<dyn Fn() -> ScenarioFixture> = match name {
        "fairness" | "round_robin_fairness" => {
            Box::new(|| fixtures::round_robin_fairness_scenario(3, 5))
        }
        "conflict" | "write_conflict" => Box::new(|| fixtures::write_conflict_scenario(3)),
        "mailbox" | "mailbox_roundtrip" => Box::new(|| fixtures::mailbox_roundtrip_scenario(0x42)),
        "dma" | "dma_block_unblock" => Box::new(fixtures::dma_block_unblock_scenario),
        "send" | "mailbox_send" => Box::new(|| fixtures::mailbox_send_scenario(5)),
        "signal" | "signal_update" => Box::new(|| fixtures::signal_update_scenario(4)),
        "isa" | "fake_isa" => Box::new(fixtures::fake_isa_scenario),
        _ => return None,
    };
    Some(factory)
}

const SCENARIOS: &[&str] = &[
    "fairness", "conflict", "mailbox", "dma", "send", "signal", "isa",
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        println!("usage: cellgov_cli <scenario>");
        println!("       cellgov_cli dump <scenario>");
        println!("       cellgov_cli compare <scenario|manifest.toml> [--mode strict|memory|events|prefix] [--format human|json]");
        println!("       cellgov_cli compare <scenario|manifest.toml> --save-baseline <path>");
        println!("       cellgov_cli compare <scenario|manifest.toml> --against-baseline <path> [--mode ...] [--format ...]");
        println!("       cellgov_cli compare <manifest.toml> --baselines-dir <dir> [--mode ...] [--format ...]");
        println!();
        println!("available scenarios:");
        for name in SCENARIOS {
            println!("  {name}");
        }
        std::process::exit(0);
    }

    if args[1] == "compare" {
        let target = args.get(2).map(String::as_str).unwrap_or_else(|| {
            eprintln!(
                "usage: cellgov_cli compare <scenario|manifest.toml> [--mode strict|memory|events|prefix]"
            );
            std::process::exit(1);
        });
        let mode = parse_compare_mode(&args);
        let format = parse_output_format(&args);
        let save_path = find_flag_value(&args, "--save-baseline");
        let against_path = find_flag_value(&args, "--against-baseline");
        let baselines_dir = find_flag_value(&args, "--baselines-dir");

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
                None => {
                    eprintln!("unknown scenario: {target}");
                    eprintln!("available: {}", SCENARIOS.join(", "));
                    std::process::exit(1);
                }
            }
        }
        return;
    }

    if args[1] == "dump" {
        let name = args.get(2).map(String::as_str).unwrap_or_else(|| {
            eprintln!("usage: cellgov_cli dump <scenario>");
            std::process::exit(1);
        });
        match run_scenario(name) {
            Some((_label, result)) => dump_trace(&result),
            None => {
                eprintln!("unknown scenario: {name}");
                eprintln!("available: {}", SCENARIOS.join(", "));
                std::process::exit(1);
            }
        }
        return;
    }

    let name = &args[1];
    match run_scenario(name) {
        Some((label, result)) => println!("{}", report(label, &result)),
        None => {
            eprintln!("unknown scenario: {name}");
            eprintln!("available: {}", SCENARIOS.join(", "));
            std::process::exit(1);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Human,
    Json,
}

/// Parse `--format human|json` from CLI args. Defaults to `Human`.
fn parse_output_format(args: &[String]) -> OutputFormat {
    for (i, arg) in args.iter().enumerate() {
        if arg == "--format" {
            if let Some(val) = args.get(i + 1) {
                return match val.as_str() {
                    "human" => OutputFormat::Human,
                    "json" => OutputFormat::Json,
                    other => {
                        eprintln!("unknown output format: {other}");
                        eprintln!("valid formats: human, json");
                        std::process::exit(1);
                    }
                };
            }
        }
    }
    OutputFormat::Human
}

/// Parse `--mode <mode>` from CLI args. Defaults to `Memory`.
fn parse_compare_mode(args: &[String]) -> CompareMode {
    for (i, arg) in args.iter().enumerate() {
        if arg == "--mode" {
            if let Some(val) = args.get(i + 1) {
                return match val.as_str() {
                    "strict" => CompareMode::Strict,
                    "memory" => CompareMode::Memory,
                    "events" => CompareMode::Events,
                    "prefix" => CompareMode::Prefix,
                    other => {
                        eprintln!("unknown compare mode: {other}");
                        eprintln!("valid modes: strict, memory, events, prefix");
                        std::process::exit(1);
                    }
                };
            }
        }
    }
    CompareMode::Memory
}

/// Find a `--flag <value>` pair in args.
fn find_flag_value(args: &[String], flag: &str) -> Option<String> {
    for (i, arg) in args.iter().enumerate() {
        if arg == flag {
            return args.get(i + 1).cloned();
        }
    }
    None
}

/// Run a scenario, observe it with determinism check, and save to disk.
fn save_baseline(factory: &dyn Fn() -> ScenarioFixture, name: &str, path: &str) {
    let regions: Vec<RegionDescriptor> = vec![];
    match observe_with_determinism_check(factory, &regions) {
        Ok(obs) => {
            let p = std::path::Path::new(path);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            cellgov_compare::baseline::save(&obs, p).unwrap_or_else(|e| {
                eprintln!("failed to save baseline: {e:?}");
                std::process::exit(1);
            });
            println!("saved baseline for {name} to {path}");
        }
        Err(e) => {
            eprintln!("determinism check FAILED for {name}: {e:?}");
            std::process::exit(1);
        }
    }
}

/// Run a scenario, observe it, load a saved baseline, and compare.
fn compare_against_baseline(
    factory: &dyn Fn() -> ScenarioFixture,
    name: &str,
    path: &str,
    mode: CompareMode,
    format: OutputFormat,
) {
    let regions: Vec<RegionDescriptor> = vec![];
    let obs = match observe_with_determinism_check(factory, &regions) {
        Ok(obs) => obs,
        Err(e) => {
            eprintln!("determinism check FAILED for {name}: {e:?}");
            std::process::exit(1);
        }
    };

    let baseline =
        cellgov_compare::baseline::load(std::path::Path::new(path)).unwrap_or_else(|e| {
            eprintln!("failed to load baseline from {path}: {e:?}");
            std::process::exit(1);
        });

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
    let regions: Vec<RegionDescriptor> = vec![];
    match observe_with_determinism_check(factory, &regions) {
        Ok(obs) => match format {
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
        },
        Err(e) => {
            eprintln!("determinism check FAILED for {name}: {e:?}");
            std::process::exit(1);
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
        .unwrap_or_else(|e| {
            eprintln!("failed to load manifest {manifest_path}: {e:?}");
            std::process::exit(1);
        });

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
            // No CellGov scenario -- classify as unsupported.
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

    let obs = match observe_with_determinism_check(&factory, &regions) {
        Ok(obs) => obs,
        Err(e) => {
            eprintln!("determinism check FAILED for {test_name}: {e:?}");
            std::process::exit(1);
        }
    };

    if let Some(dir) = baselines_dir {
        let baselines = load_baselines_from_dir(&dir);
        if baselines.is_empty() {
            eprintln!("no baseline .json files found in {dir}");
            std::process::exit(1);
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
        let baseline =
            cellgov_compare::baseline::load(std::path::Path::new(&path)).unwrap_or_else(|e| {
                eprintln!("failed to load baseline from {path}: {e:?}");
                std::process::exit(1);
            });

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
        // No baseline to compare against -- just print the observation.
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
fn load_baselines_from_dir(dir: &str) -> Vec<Observation> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| {
            eprintln!("failed to read baselines directory {dir}: {e}");
            std::process::exit(1);
        })
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                Some(path)
            } else {
                None
            }
        })
        .collect();
    entries.sort();

    entries
        .iter()
        .map(|path| {
            cellgov_compare::baseline::load(path).unwrap_or_else(|e| {
                eprintln!("failed to load baseline {}: {e:?}", path.display());
                std::process::exit(1);
            })
        })
        .collect()
}

/// Print every trace record from a scenario run, one per line.
fn dump_trace(result: &ScenarioResult) {
    use cellgov_trace::{TraceRecord, TracedBlockReason, TracedWakeReason};

    for (i, rec) in TraceReader::new(&result.trace_bytes)
        .map(|r| r.expect("trace decode failed"))
        .enumerate()
    {
        match rec {
            TraceRecord::UnitScheduled {
                unit,
                granted_budget,
                time,
                epoch,
            } => {
                println!(
                    "{i:4}  UnitScheduled      unit={} budget={} time={} epoch={}",
                    unit.raw(),
                    granted_budget.raw(),
                    time.raw(),
                    epoch.raw()
                );
            }
            TraceRecord::StepCompleted {
                unit,
                yield_reason,
                consumed_budget,
                time_after,
            } => {
                println!(
                    "{i:4}  StepCompleted      unit={} yield={:?} consumed={} time_after={}",
                    unit.raw(),
                    yield_reason,
                    consumed_budget.raw(),
                    time_after.raw()
                );
            }
            TraceRecord::EffectEmitted {
                unit,
                sequence,
                kind,
            } => {
                println!(
                    "{i:4}  EffectEmitted      unit={} seq={} kind={:?}",
                    unit.raw(),
                    sequence,
                    kind
                );
            }
            TraceRecord::CommitApplied {
                unit,
                writes_committed,
                effects_deferred,
                fault_discarded,
                epoch_after,
            } => {
                println!(
                    "{i:4}  CommitApplied      unit={} writes={} deferred={} fault={} epoch_after={}",
                    unit.raw(),
                    writes_committed,
                    effects_deferred,
                    fault_discarded,
                    epoch_after.raw()
                );
            }
            TraceRecord::StateHashCheckpoint { kind, hash } => {
                println!(
                    "{i:4}  StateHashCheckpoint kind={:?} hash=0x{:016x}",
                    kind,
                    hash.raw()
                );
            }
            TraceRecord::UnitBlocked { unit, reason } => {
                let reason_str = match reason {
                    TracedBlockReason::WaitOnEvent => "WaitOnEvent",
                    TracedBlockReason::MailboxEmpty => "MailboxEmpty",
                };
                println!(
                    "{i:4}  UnitBlocked        unit={} reason={}",
                    unit.raw(),
                    reason_str
                );
            }
            TraceRecord::UnitWoken { unit, reason } => {
                let reason_str = match reason {
                    TracedWakeReason::WakeEffect => "WakeEffect",
                    TracedWakeReason::DmaCompletion => "DmaCompletion",
                };
                println!(
                    "{i:4}  UnitWoken          unit={} reason={}",
                    unit.raw(),
                    reason_str
                );
            }
        }
    }
    let count = TraceReader::new(&result.trace_bytes).count();
    println!("--- {count} records total ---");
}

/// Format a [`ScenarioResult`] as a deterministic, ASCII-only summary.
fn report(name: &str, result: &ScenarioResult) -> String {
    let outcome = match result.outcome {
        ScenarioOutcome::Stalled => "Stalled",
        ScenarioOutcome::MaxStepsExceeded => "MaxStepsExceeded",
    };
    format!(
        "scenario: {name}\noutcome: {outcome}\nsteps_taken: {steps}\ntrace_bytes: {bytes}\nmemory_hash: 0x{mem:016x}\nstatus_hash: 0x{status:016x}\nsync_hash: 0x{sync:016x}",
        steps = result.steps_taken,
        bytes = result.trace_bytes.len(),
        mem = result.final_memory_hash.raw(),
        status = result.final_unit_status_hash.raw(),
        sync = result.final_sync_hash.raw(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_named_scenario_runs_successfully() {
        for name in SCENARIOS {
            let (label, result) =
                run_scenario(name).unwrap_or_else(|| panic!("scenario {name} not found"));
            assert_eq!(
                result.outcome,
                ScenarioOutcome::Stalled,
                "scenario {label} did not stall cleanly"
            );
            assert!(result.steps_taken > 0, "scenario {label} took zero steps");
        }
    }

    #[test]
    fn unknown_scenario_returns_none() {
        assert!(run_scenario("nonexistent").is_none());
    }

    #[test]
    fn report_is_deterministic_across_runs() {
        let (l1, r1) = run_scenario("fairness").unwrap();
        let (l2, r2) = run_scenario("fairness").unwrap();
        assert_eq!(report(l1, &r1), report(l2, &r2));
    }

    #[test]
    fn dump_does_not_panic_for_any_scenario() {
        for name in SCENARIOS {
            let (_, result) =
                run_scenario(name).unwrap_or_else(|| panic!("scenario {name} not found"));
            // Just verify decoding succeeds for every record.
            let records: Vec<_> = TraceReader::new(&result.trace_bytes)
                .map(|r| r.expect("decode"))
                .collect();
            assert!(
                !records.is_empty(),
                "scenario {name} produced no trace records"
            );
        }
    }

    #[test]
    fn report_includes_sync_hash_field() {
        let (label, result) = run_scenario("isa").unwrap();
        let r = report(label, &result);
        assert!(r.contains("sync_hash: 0x"));
    }

    #[test]
    fn compare_succeeds_for_every_scenario() {
        for name in SCENARIOS {
            let factory =
                scenario_factory(name).unwrap_or_else(|| panic!("scenario {name} not found"));
            let result = observe_with_determinism_check(&factory, &[]);
            assert!(
                result.is_ok(),
                "compare failed for {name}: {:?}",
                result.err()
            );
        }
    }

    #[test]
    fn parse_compare_mode_defaults_to_memory() {
        let args: Vec<String> = vec!["cli".into(), "compare".into(), "isa".into()];
        assert_eq!(parse_compare_mode(&args), CompareMode::Memory);
    }

    #[test]
    fn parse_compare_mode_reads_flag() {
        let args: Vec<String> = vec![
            "cli".into(),
            "compare".into(),
            "isa".into(),
            "--mode".into(),
            "strict".into(),
        ];
        assert_eq!(parse_compare_mode(&args), CompareMode::Strict);
    }

    #[test]
    fn parse_output_format_defaults_to_human() {
        let args: Vec<String> = vec!["cli".into(), "compare".into(), "isa".into()];
        assert_eq!(parse_output_format(&args), OutputFormat::Human);
    }

    #[test]
    fn parse_output_format_reads_json_flag() {
        let args: Vec<String> = vec![
            "cli".into(),
            "compare".into(),
            "isa".into(),
            "--format".into(),
            "json".into(),
        ];
        assert_eq!(parse_output_format(&args), OutputFormat::Json);
    }

    #[test]
    fn scenario_factory_accepts_full_names() {
        // Manifests use full fixture names like "mailbox_send".
        assert!(scenario_factory("mailbox_send").is_some());
        assert!(scenario_factory("dma_block_unblock").is_some());
        assert!(scenario_factory("fake_isa").is_some());
        assert!(scenario_factory("round_robin_fairness").is_some());
        assert!(scenario_factory("write_conflict").is_some());
        assert!(scenario_factory("signal_update").is_some());
        assert!(scenario_factory("mailbox_roundtrip").is_some());
    }

    #[test]
    fn scenario_factory_returns_none_for_unknown() {
        assert!(scenario_factory("nonexistent").is_none());
    }
}
