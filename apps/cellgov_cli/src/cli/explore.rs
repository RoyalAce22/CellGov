//! `explore` subcommand: bounded schedule exploration for both
//! testkit scenarios and LV2-driven ELF microtests.

use cellgov_explore::ExplorationConfig;
use cellgov_testkit::fixtures::ScenarioFixture;

use super::args::{find_flag_value, parse_output_format, OutputFormat};
use super::compare::load_baselines_from_dir;
use super::exit::{die, load_file_or_die};
use super::scenarios::{build_lv2_fixture, microtest_region_defs, scenario_factory, MICROTESTS};

/// Entry point for `cellgov_cli explore ...`.
pub(crate) fn run(args: &[String], scenarios_list: &[&str]) {
    let target = args.get(2).map(String::as_str).unwrap_or_else(|| {
        die(
            "usage: cellgov_cli explore <scenario> [--format human|json]\n       cellgov_cli explore micro <name> [--format human|json]",
        )
    });
    let format = parse_output_format(args);
    if target == "micro" {
        let name = args.get(3).map(String::as_str).unwrap_or_else(|| {
            die(&format!(
                "usage: cellgov_cli explore micro <name> [--format human|json]\n       cellgov_cli explore micro <name> --baselines-dir <dir> [--format human|json]\navailable microtests: {}",
                MICROTESTS.join(", ")
            ))
        });
        let baselines_dir = find_flag_value(args, "--baselines-dir");
        if let Some(dir) = baselines_dir {
            run_explore_micro_oracle(name, &dir, format);
        } else {
            run_explore_micro(name, format);
        }
    } else {
        match scenario_factory(target) {
            Some(factory) => run_explore(&factory, target, format),
            None => die(&format!(
                "unknown scenario: {target}\navailable: {}",
                scenarios_list.join(", ")
            )),
        }
    }
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

/// Run oracle-aware schedule exploration on an LV2-driven microtest.
fn run_explore_micro_oracle(name: &str, baselines_dir: &str, format: OutputFormat) {
    if !MICROTESTS.contains(&name) {
        die(&format!(
            "unknown microtest: {name}\navailable: {}",
            MICROTESTS.join(", ")
        ));
    }

    let baselines = load_baselines_from_dir(baselines_dir);
    if baselines.is_empty() {
        die(&format!("no baseline .json files found in {baselines_dir}"));
    }

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
    baselines: &[cellgov_compare::Observation],
) -> bool {
    baselines.iter().any(|oracle| {
        captured.iter().all(|region| {
            oracle.memory_regions.iter().any(|oracle_region| {
                oracle_region.name == region.name && oracle_region.data == region.data
            })
        })
    })
}
