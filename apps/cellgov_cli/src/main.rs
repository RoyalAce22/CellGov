//! cellgov_cli -- run scenarios, dump traces, compare observations, explore schedules.
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
//! - `cellgov_cli explore <scenario> [--format human|json]` --
//!   run bounded schedule exploration on a scenario and report classification.
//!
//! Available scenarios: fairness, conflict, mailbox, dma, send, signal, isa.

use cellgov_compare::{
    compare, compare_multi, format_human, format_json, format_multi_human, format_multi_json,
    observe_with_determinism_check, Classification, CompareMode, Observation, RegionDescriptor,
};
use cellgov_explore::ExplorationConfig;
use cellgov_testkit::fixtures::{self, ScenarioFixture};
use cellgov_testkit::runner::{run, ScenarioOutcome, ScenarioResult};
use cellgov_trace::TraceReader;

mod game;

// -- CLI helpers --

fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1)
}

fn load_file_or_die(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| die(&format!("failed to read {path}: {e}")))
}

fn require_determinism(
    factory: &dyn Fn() -> ScenarioFixture,
    name: &str,
    regions: &[RegionDescriptor],
) -> Observation {
    observe_with_determinism_check(factory, regions)
        .unwrap_or_else(|e| die(&format!("determinism check FAILED for {name}: {e:?}")))
}

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

const MICROTESTS: &[&str] = &["barrier_wakeup", "mailbox_roundtrip", "atomic_reservation"];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        println!("usage: cellgov_cli <scenario>");
        println!("       cellgov_cli dump <scenario>");
        println!("       cellgov_cli compare <scenario|manifest.toml> [--mode strict|memory|events|prefix] [--format human|json]");
        println!("       cellgov_cli compare <scenario|manifest.toml> --save-baseline <path>");
        println!("       cellgov_cli compare <scenario|manifest.toml> --against-baseline <path> [--mode ...] [--format ...]");
        println!("       cellgov_cli compare <manifest.toml> --baselines-dir <dir> [--mode ...] [--format ...]");
        println!("       cellgov_cli explore <scenario> [--format human|json]");
        println!("       cellgov_cli explore micro <name> [--format human|json]");
        println!("       cellgov_cli run-game <elf-path> [--max-steps N] [--trace] [--profile]");
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

    if args[1] == "explore" {
        let target = args.get(2).map(String::as_str).unwrap_or_else(|| {
            eprintln!("usage: cellgov_cli explore <scenario> [--format human|json]");
            eprintln!("       cellgov_cli explore micro <name> [--format human|json]");
            std::process::exit(1);
        });
        let format = parse_output_format(&args);
        if target == "micro" {
            let name = args.get(3).map(String::as_str).unwrap_or_else(|| {
                eprintln!("usage: cellgov_cli explore micro <name> [--format human|json]");
                eprintln!("       cellgov_cli explore micro <name> --baselines-dir <dir> [--format human|json]");
                eprintln!("available microtests: {}", MICROTESTS.join(", "));
                std::process::exit(1);
            });
            let baselines_dir = find_flag_value(&args, "--baselines-dir");
            if let Some(dir) = baselines_dir {
                run_explore_micro_oracle(name, &dir, format);
            } else {
                run_explore_micro(name, format);
            }
        } else {
            match scenario_factory(target) {
                Some(factory) => run_explore(&factory, target, format),
                None => {
                    eprintln!("unknown scenario: {target}");
                    eprintln!("available: {}", SCENARIOS.join(", "));
                    std::process::exit(1);
                }
            }
        }
        return;
    }

    if args[1] == "run-game" {
        let elf_path = args.get(2).map(String::as_str).unwrap_or_else(|| {
            eprintln!(
                "usage: cellgov_cli run-game <elf-path> [--max-steps N] [--trace] [--profile]"
            );
            std::process::exit(1);
        });
        let max_steps: usize = find_flag_value(&args, "--max-steps")
            .and_then(|v| v.parse().ok())
            .unwrap_or(100_000);
        let trace = args.iter().any(|a| a == "--trace");
        let profile = args.iter().any(|a| a == "--profile");
        game::run_game(elf_path, max_steps, trace, profile);
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
    let obs = require_determinism(factory, name, &[]);
    let p = std::path::Path::new(path);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).ok();
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

/// Build a ScenarioFixture for an LV2-driven ELF microtest.
///
/// Reads PPU and SPU ELF binaries from `tests/micro/<name>/build/`,
/// then constructs a fixture that boots the PPU, which drives SPU
/// creation via LV2 syscalls.
fn build_lv2_fixture(name: &str) -> ScenarioFixture {
    use cellgov_mem::{ByteRange, GuestAddr};
    use cellgov_ppu::PpuExecutionUnit;
    use cellgov_spu::{loader as spu_loader, SpuExecutionUnit};
    use cellgov_time::Budget;
    use std::cell::RefCell;
    use std::rc::Rc;

    let base = format!("tests/micro/{name}/build");
    let ppu_elf = load_file_or_die(&format!("{base}/{name}.elf"));
    let spu_elf = load_file_or_die(&format!("{base}/spu_main.elf"));

    let mem_size = 0x1002_0000usize;
    let stack_top = (mem_size as u64) - 0x1000;
    let primed: Rc<RefCell<Option<cellgov_ppu::state::PpuState>>> = Rc::new(RefCell::new(None));
    let primed_seed = Rc::clone(&primed);
    let primed_reg = Rc::clone(&primed);

    ScenarioFixture::builder()
        .memory_size(mem_size)
        .budget(Budget::new(100_000))
        .max_steps(10_000)
        .seed_memory(move |mem| {
            let li_r11_22: u32 = (14 << 26) | (11 << 21) | 22;
            let sc: u32 = 0x4400_0002;
            let stub_range = ByteRange::new(GuestAddr::new(0), 8).unwrap();
            let mut stub_bytes = Vec::with_capacity(8);
            stub_bytes.extend_from_slice(&li_r11_22.to_be_bytes());
            stub_bytes.extend_from_slice(&sc.to_be_bytes());
            mem.apply_commit(stub_range, &stub_bytes).unwrap();

            let mut state = cellgov_ppu::state::PpuState::new();
            cellgov_ppu::loader::load_ppu_elf(&ppu_elf, mem, &mut state).unwrap();
            state.gpr[1] = stack_top;
            state.lr = 0;
            *primed_seed.borrow_mut() = Some(state);
        })
        .register(move |rt| {
            rt.lv2_host_mut()
                .content_store_mut()
                .register(b"/app_home/spu_main.elf", spu_elf.clone());

            rt.set_spu_factory(move |id, init| {
                let mut unit = SpuExecutionUnit::new(id);
                spu_loader::load_spu_elf(&init.ls_bytes, unit.state_mut()).unwrap();
                unit.state_mut().pc = init.entry_pc;
                unit.state_mut().set_reg_word_splat(1, init.stack_ptr);
                unit.state_mut().set_reg_word_splat(3, init.args[0] as u32);
                unit.state_mut().set_reg_word_splat(4, init.args[1] as u32);
                unit.state_mut().set_reg_word_splat(5, init.args[2] as u32);
                unit.state_mut().set_reg_word_splat(6, init.args[3] as u32);
                Box::new(unit)
            });

            let ppu_state = primed_reg.borrow_mut().take().unwrap();
            rt.registry_mut().register_with(|id| {
                let mut unit = PpuExecutionUnit::new(id);
                *unit.state_mut() = ppu_state;
                unit
            });
        })
        .build()
}

/// Run bounded schedule exploration on an LV2-driven ELF microtest.
fn run_explore_micro(name: &str, format: OutputFormat) {
    if !MICROTESTS.contains(&name) {
        die(&format!(
            "unknown microtest: {name}\navailable: {}",
            MICROTESTS.join(", ")
        ));
    }
    let config = ExplorationConfig::default();
    let result = cellgov_explore::explore(|| build_lv2_fixture(name).build_runtime(), &config);
    match result {
        Some(r) => {
            match format {
                OutputFormat::Human => {
                    println!("microtest: {name}");
                    print!("{}", cellgov_explore::report::format_human(&r));
                }
                OutputFormat::Json => {
                    println!("{}", cellgov_explore::report::format_json(&r));
                }
            }
            if r.outcome == cellgov_explore::OutcomeClass::ScheduleSensitive {
                std::process::exit(1);
            }
        }
        None => {
            println!("microtest: {name}");
            println!("outcome: no branching points (single-unit or trivial)");
        }
    }
}

/// Region specs for each microtest: (symbol_name, [(region_name, offset, size)]).
fn microtest_region_defs(name: &str) -> (&str, Vec<(&str, u64, u64)>) {
    match name {
        "barrier_wakeup" => ("buf", vec![("spu0_result", 0, 8), ("spu1_result", 16, 8)]),
        "mailbox_roundtrip" => ("result", vec![("result", 0, 8)]),
        "atomic_reservation" => ("buf", vec![("header", 0, 8), ("data", 16, 128)]),
        _ => {
            eprintln!("no region defs for microtest: {name}");
            std::process::exit(1);
        }
    }
}

/// Run oracle-aware schedule exploration on an LV2-driven microtest.
fn run_explore_micro_oracle(name: &str, baselines_dir: &str, format: OutputFormat) {
    if !MICROTESTS.contains(&name) {
        die(&format!(
            "unknown microtest: {name}\navailable: {}",
            MICROTESTS.join(", ")
        ));
    }

    // Load oracle baselines.
    let baselines = load_baselines_from_dir(baselines_dir);
    if baselines.is_empty() {
        eprintln!("no baseline .json files found in {baselines_dir}");
        std::process::exit(1);
    }

    // Resolve memory regions from ELF symbol.
    let (symbol, region_defs) = microtest_region_defs(name);
    let base = format!("tests/micro/{name}/build");
    let ppu_elf = load_file_or_die(&format!("{base}/{name}.elf"));
    let base_addr = cellgov_ppu::loader::find_symbol(&ppu_elf, symbol)
        .unwrap_or_else(|| die(&format!("symbol '{symbol}' not found in {base}/{name}.elf")));

    let region_specs: Vec<cellgov_explore::MemoryRegionSpec> = region_defs
        .iter()
        .map(|(rname, offset, size)| cellgov_explore::MemoryRegionSpec {
            name: (*rname).into(),
            addr: base_addr + offset,
            size: *size,
        })
        .collect();

    // Run oracle-aware exploration.
    let config = ExplorationConfig::default();
    let result = cellgov_explore::explore_with_regions(
        || build_lv2_fixture(name).build_runtime(),
        &config,
        &region_specs,
    );

    let Some(r) = result else {
        println!("microtest: {name}");
        println!("outcome: no branching points");
        return;
    };

    // Compare each schedule's regions against oracle baselines.
    let baseline_matches = compare_regions_against_oracle(&r.baseline.regions, &baselines);
    let alt_matches: Vec<bool> = r
        .alternates
        .iter()
        .map(|s| compare_regions_against_oracle(&s.regions, &baselines))
        .collect();

    let all_match = baseline_matches && alt_matches.iter().all(|m| *m);
    let any_match = baseline_matches || alt_matches.iter().any(|m| *m);

    match format {
        OutputFormat::Human => {
            println!("microtest: {name}");
            print!("{}", cellgov_explore::report::format_human(&r.exploration));
            println!("oracle_baselines: {}", baselines.len());
            println!("baseline_matches_oracle: {baseline_matches}");
            for (i, m) in alt_matches.iter().enumerate() {
                if !m {
                    println!("  schedule {i}: ORACLE MISMATCH");
                }
            }
            if all_match {
                println!("oracle_verdict: all schedules match oracle");
            } else if any_match {
                println!("oracle_verdict: PARTIAL -- some schedules diverge from oracle");
            } else {
                println!("oracle_verdict: NONE -- no schedule matches oracle");
            }
        }
        OutputFormat::Json => {
            let json = serde_json::json!({
                "exploration": serde_json::from_str::<serde_json::Value>(
                    &cellgov_explore::report::format_json(&r.exploration)
                ).unwrap(),
                "oracle": {
                    "baselines_count": baselines.len(),
                    "baseline_matches": baseline_matches,
                    "alternate_matches": alt_matches,
                    "all_match": all_match,
                    "any_match": any_match,
                },
            });
            println!("{}", serde_json::to_string_pretty(&json).unwrap());
        }
    }

    if !all_match {
        std::process::exit(1);
    }
}

/// Compare captured regions against oracle baselines.
/// Returns true if all regions match at least one baseline.
fn compare_regions_against_oracle(
    captured: &[cellgov_explore::oracle::CapturedRegion],
    baselines: &[Observation],
) -> bool {
    // Match against ANY baseline (oracle agreement not checked here;
    // that is the compare crate's concern).
    baselines.iter().any(|oracle| {
        captured.iter().all(|region| {
            oracle.memory_regions.iter().any(|oracle_region| {
                oracle_region.name == region.name && oracle_region.data == region.data
            })
        })
    })
}

/// Run bounded schedule exploration on a testkit scenario.
fn run_explore(factory: &dyn Fn() -> ScenarioFixture, name: &str, format: OutputFormat) {
    let config = ExplorationConfig::default();
    let result = cellgov_explore::explore(|| factory().build_runtime(), &config);
    match result {
        Some(r) => {
            match format {
                OutputFormat::Human => {
                    println!("scenario: {name}");
                    print!("{}", cellgov_explore::report::format_human(&r));
                }
                OutputFormat::Json => {
                    println!("{}", cellgov_explore::report::format_json(&r));
                }
            }
            if r.outcome == cellgov_explore::OutcomeClass::ScheduleSensitive {
                std::process::exit(1);
            }
        }
        None => {
            println!("scenario: {name}");
            println!("outcome: no branching points (single-unit or trivial)");
        }
    }
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

    #[test]
    fn explore_runs_for_multi_unit_scenarios() {
        // Scenarios with >1 unit should produce exploration results.
        for name in &["fairness", "conflict", "mailbox"] {
            let factory =
                scenario_factory(name).unwrap_or_else(|| panic!("scenario {name} not found"));
            let config = ExplorationConfig::default();
            let result = cellgov_explore::explore(|| factory().build_runtime(), &config);
            assert!(
                result.is_some(),
                "scenario {name} should have branching points"
            );
        }
    }

    #[test]
    fn explore_single_unit_returns_none() {
        let factory = scenario_factory("isa").expect("isa scenario exists");
        let config = ExplorationConfig::default();
        let result = cellgov_explore::explore(|| factory().build_runtime(), &config);
        assert!(result.is_none(), "single-unit isa has no branching points");
    }

    #[test]
    #[ignore] // ~7 min: runs 3 ELF microtests with full exploration
    fn explore_micro_runs_for_elf_microtests() {
        // The test binary's CWD may vary; use the CARGO_MANIFEST_DIR
        // to locate the repo-root-relative test fixtures.
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let repo_root = std::path::Path::new(&manifest_dir)
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let config = ExplorationConfig::default();
        for name in MICROTESTS {
            let base = repo_root.join(format!("tests/micro/{name}/build"));
            let ppu_path = base.join(format!("{name}.elf"));
            let spu_path = base.join("spu_main.elf");
            if !ppu_path.exists() || !spu_path.exists() {
                continue; // ELFs not built; skip
            }
            // Temporarily chdir to repo root so build_lv2_fixture
            // can find the ELFs with its relative paths.
            let prev = std::env::current_dir().unwrap();
            std::env::set_current_dir(repo_root).unwrap();
            let result =
                cellgov_explore::explore(|| build_lv2_fixture(name).build_runtime(), &config);
            std::env::set_current_dir(&prev).unwrap();
            assert!(
                result.is_some(),
                "microtest {name} should have branching points"
            );
            let r = result.unwrap();
            assert_eq!(
                r.outcome,
                cellgov_explore::OutcomeClass::ScheduleStable,
                "microtest {name} should be schedule-stable"
            );
        }
    }
}
